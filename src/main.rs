#![forbid(unsafe_code)]

mod cli;
mod install;
mod platform;

use std::collections::HashMap;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::sync::{Arc, Mutex};

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use std::thread;
use std::time::{Duration, Instant};

use hebrew_tty::classify::{
    is_recorded_agent, select_mode, Classifier, ExecutionPath, Host, ObservedEvidence,
};
use hebrew_tty::config::{Config, Mode};
use hebrew_tty::diagnostics::{DiagnosticRecord, Diagnostics};

use platform::foreground::Foreground;
use platform::{Classified, ForegroundClassifier, Launch};

fn detect_agent_version(program: &std::ffi::OsStr, agent_name: &std::ffi::OsStr) -> Option<String> {
    let name = Path::new(agent_name).file_name()?.to_str()?;
    if !is_recorded_agent(name) {
        return None;
    }
    let mut command = ProcessCommand::new(program);
    command
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut probe = ProbeChild(command.spawn().ok()?);
    let mut stdout = probe.0.stdout.take()?;
    let mut stderr = probe.0.stderr.take()?;
    set_nonblocking(stdout.as_raw_fd())?;
    set_nonblocking(stderr.as_raw_fd())?;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if !drain_version_capture(&mut stdout, &mut stdout_bytes)
            || !drain_version_capture(&mut stderr, &mut stderr_bytes)
        {
            return None;
        }
        match probe.0.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(_) => return None,
        }
        if Instant::now() >= deadline {
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    };
    terminate_probe_group(&probe.0);
    if !drain_version_capture(&mut stdout, &mut stdout_bytes)
        || !drain_version_capture(&mut stderr, &mut stderr_bytes)
        || !status.success()
    {
        return None;
    }
    let stdout_text = String::from_utf8(stdout_bytes).ok()?;
    let stderr_text = String::from_utf8(stderr_bytes).ok()?;
    let version = if stdout_text.trim().is_empty() {
        stderr_text.trim()
    } else {
        stdout_text.trim()
    };
    (!version.is_empty()).then(|| version.to_owned())
}

fn set_nonblocking(fd: std::os::fd::RawFd) -> Option<()> {
    let flags = OFlag::from_bits_truncate(fcntl(fd, FcntlArg::F_GETFL).ok()?);
    fcntl(fd, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK)).ok()?;
    Some(())
}

fn drain_version_capture(reader: &mut impl Read, bytes: &mut Vec<u8>) -> bool {
    const MAX_VERSION_BYTES: usize = 4 * 1024;
    let mut buffer = [0; 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return true,
            Ok(count) => {
                bytes.extend_from_slice(&buffer[..count]);
                if bytes.len() > MAX_VERSION_BYTES {
                    return false;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return true,
            Err(_) => return false,
        }
    }
}

struct ProbeChild(Child);

impl Drop for ProbeChild {
    fn drop(&mut self) {
        terminate_probe_group(&self.0);
        let _ = self.0.wait();
    }
}

fn terminate_probe_group(child: &Child) {
    let Some(group) = rustix::process::Pid::from_raw(child.id() as i32) else {
        return;
    };
    let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
}

fn open_diagnostics(path: &std::ffi::OsStr) -> io::Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .custom_flags(nix::libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "diagnostics path must be a regular file",
        ));
    }
    for stream in ["/proc/self/fd/1", "/proc/self/fd/2"] {
        if let Ok(stream_metadata) = std::fs::metadata(stream) {
            if metadata.dev() == stream_metadata.dev() && metadata.ino() == stream_metadata.ino() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "diagnostics path must not alias stdout or stderr",
                ));
            }
        }
    }
    Ok(file)
}

/// Classifies whatever the inner pty brings to the foreground, the way the
/// launch command is classified, with each `--version` asked once per program.
struct AgentClassifier {
    config: Config,
    host: Host,
    cli_mode: Option<Mode>,
    diagnostics: Option<Mutex<Diagnostics<File>>>,
    reported: Mutex<Option<String>>,
    versions: Mutex<HashMap<(String, OsString), Option<String>>>,
}

impl AgentClassifier {
    /// One record per change of verdict. The first thing the pty brings to
    /// the foreground is the launch command itself, already reported.
    fn report(&self, name: &str, path: &ExecutionPath, mode: Mode) -> io::Result<()> {
        let Some(diagnostics) = &self.diagnostics else {
            return Ok(());
        };
        let selection = select_mode(mode, path);
        let key = format!(
            "{name}\0{:?}\0{:?}\0{mode}",
            path.confidence, selection.disposition
        );
        let Ok(mut reported) = self.reported.lock() else {
            return Ok(());
        };
        if reported.as_deref() == Some(key.as_str()) {
            return Ok(());
        }
        *reported = Some(key);
        let record = DiagnosticRecord::new(name, Some(self.host), mode, path, &selection);
        match diagnostics.lock() {
            Ok(mut diagnostics) => diagnostics.emit(&record),
            Err(_) => Ok(()),
        }
    }

    fn version(&self, name: &str, program: &OsStr) -> Option<String> {
        if !is_recorded_agent(name) {
            return None;
        }
        let key = (name.to_owned(), program.to_owned());
        if let Some(cached) = self
            .versions
            .lock()
            .ok()
            .and_then(|cache| cache.get(&key).cloned())
        {
            return cached;
        }
        let version = detect_agent_version(program, OsStr::new(name));
        if let Ok(mut cache) = self.versions.lock() {
            cache.insert(key, version.clone());
        }
        version
    }

    fn verdict(&self, name: &str, program: &OsStr) -> Classified {
        let path = Classifier.observe(
            OsStr::new(name),
            Some(self.host),
            ObservedEvidence {
                agent_version: self.version(name, program),
                ..ObservedEvidence::default()
            },
        );
        let mode = self.config.policy_for(OsStr::new(name), self.cli_mode).mode;
        let _ = self.report(name, &path, mode);
        Classified {
            name: name.to_owned(),
            path,
            mode,
        }
    }
}

impl ForegroundClassifier for AgentClassifier {
    fn classify(&self, foreground: &Foreground) -> Classified {
        let candidates = foreground.candidates();
        match candidates
            .iter()
            .find(|candidate| is_recorded_agent(&candidate.name))
        {
            Some(candidate) => self.verdict(&candidate.name, &candidate.program),
            None => self.verdict(&foreground.display_name(), OsStr::new("")),
        }
    }
}

fn default_shell() -> cli::Command {
    let program = std::env::var_os("SHELL")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| OsString::from("/bin/sh"));
    cli::Command {
        program,
        args: Vec::new(),
        argv0: None,
    }
}

fn run(invocation: cli::Invocation) -> Result<i32, Box<dyn Error>> {
    let config = Config::load()?;
    let host = match std::env::var("HEBREW_TTY_HOST").as_deref() {
        Ok("herdr") => Host::Herdr,
        _ => Host::Direct,
    };
    let command = invocation.command.unwrap_or_else(default_shell);
    let follow_name = command.argv0.is_none();
    let policy = config.policy_for(&command.program, invocation.mode);
    let diagnostics = match invocation.diagnostics {
        Some(path) => Some(Mutex::new(Diagnostics::new(open_diagnostics(&path)?))),
        None => None,
    };
    let classifier = Arc::new(AgentClassifier {
        config,
        host,
        cli_mode: invocation.mode,
        diagnostics,
        reported: Mutex::new(None),
        versions: Mutex::new(HashMap::new()),
    });
    let agent_name = command
        .argv0
        .clone()
        .unwrap_or_else(|| command.program.clone());
    let name = Path::new(&agent_name)
        .file_name()
        .unwrap_or(&agent_name)
        .to_string_lossy()
        .into_owned();
    let path = Classifier.observe(
        &agent_name,
        Some(host),
        ObservedEvidence {
            agent_version: classifier.version(&name, &command.program),
            ..ObservedEvidence::default()
        },
    );
    classifier.report(&name, &path, policy.mode)?;

    platform::run(Launch {
        command,
        path,
        mode: policy.mode,
        classifier,
        follow_name,
    })
}

fn finish(result: Result<String, Box<dyn Error>>) {
    match result {
        Ok(message) => println!("{message}"),
        Err(error) => {
            eprintln!("hebrew-tty: {error}");
            std::process::exit(1);
        }
    }
}

/// Under `--install` the shell exec'd into the proxy, so a proxy that cannot
/// run has to become the shell itself or the terminal closes on the error.
fn continue_in_plain_shell() {
    let shell = default_shell();
    eprintln!(
        "hebrew-tty: continuing in {} without repair",
        shell.program.to_string_lossy()
    );
    let error = ProcessCommand::new(&shell.program)
        .env("HEBREW_TTY", "1")
        .exec();
    eprintln!("hebrew-tty: {error}");
}

fn main() {
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Action::Help) => {
            print!("{}", cli::HELP);
        }
        Ok(cli::Action::Install) => finish(install::install()),
        Ok(cli::Action::Uninstall) => finish(install::uninstall()),
        Ok(cli::Action::Run(invocation)) => {
            let wraps_shell = invocation.command.is_none();
            match run(invocation) {
                Ok(code) => std::process::exit(code),
                Err(error) => {
                    eprintln!("hebrew-tty: {error}");
                    if wraps_shell {
                        continue_in_plain_shell();
                    }
                    std::process::exit(1);
                }
            }
        }
        Err(error) => {
            eprintln!("hebrew-tty: {error}\n{}", cli::USAGE);
            std::process::exit(2);
        }
    }
}

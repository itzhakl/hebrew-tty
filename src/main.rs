#![forbid(unsafe_code)]

mod cli;
mod platform;

use std::error::Error;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command as ProcessCommand, Stdio};

use nix::fcntl::{fcntl, FcntlArg, OFlag};
use std::thread;
use std::time::{Duration, Instant};

use hebrew_tty::classify::{select_mode, Classifier, Host, ObservedEvidence};
use hebrew_tty::config::Config;
use hebrew_tty::diagnostics::{DiagnosticRecord, Diagnostics};

fn detect_agent_version(program: &std::ffi::OsStr) -> Option<String> {
    let name = Path::new(program).file_name()?.to_str()?;
    if !matches!(name, "claude" | "pi" | "codex") {
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

fn run(invocation: cli::Invocation) -> Result<i32, Box<dyn Error>> {
    let config = Config::load()?;
    let policy = config.policy_for(&invocation.command.program, invocation.mode);
    let host = match std::env::var("HEBREW_TTY_HOST").as_deref() {
        Ok("herdr") => Host::Herdr,
        _ => Host::Direct,
    };
    let path = Classifier.observe(
        &invocation.command.program,
        Some(host),
        ObservedEvidence {
            agent_version: detect_agent_version(&invocation.command.program),
            ..ObservedEvidence::default()
        },
    );
    let selection = select_mode(policy.mode, &path);

    if let Some(diagnostics_path) = invocation.diagnostics {
        let command = Path::new(&invocation.command.program)
            .file_name()
            .unwrap_or(&invocation.command.program)
            .to_string_lossy();
        let file = open_diagnostics(&diagnostics_path)?;
        Diagnostics::new(file).emit(&DiagnosticRecord::new(
            &command,
            Some(host),
            policy.mode,
            &path,
            &selection,
        ))?;
    }

    platform::run(invocation.command, path, policy.mode)
}

fn main() {
    match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Action::Help) => {
            print!("{}", cli::HELP);
        }
        Ok(cli::Action::Run(invocation)) => match run(invocation) {
            Ok(code) => std::process::exit(code),
            Err(error) => {
                eprintln!("hebrew-tty: {error}");
                std::process::exit(1);
            }
        },
        Err(error) => {
            eprintln!("hebrew-tty: {error}\n{}", cli::USAGE);
            std::process::exit(2);
        }
    }
}

#![forbid(unsafe_code)]

use std::error::Error;
use std::ffi::{CString, OsString};
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use hebrew_tty::relay::{RelayWriter, Transform};
use hebrew_tty::trace::TraceWriter;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use rustix::process::{waitpid, Pid as RustixPid, WaitOptions, WaitStatus};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGWINCH};
use signal_hook::flag;
use signal_hook::iterator::Signals;

use super::foreground::{exe_of, Foreground};
use super::{Classified, ForegroundClassifier, Launch, WindowSize};
use crate::cli::Command;

struct RawModeGuard(bool);

impl RawModeGuard {
    fn enter() -> io::Result<Self> {
        if std::io::IsTerminal::is_terminal(&io::stdin()) {
            enable_raw_mode()?;
            Ok(Self(true))
        } else {
            Ok(Self(false))
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        if self.0 {
            let _ = disable_raw_mode();
        }
    }
}

fn terminal_size() -> WindowSize {
    crossterm::terminal::size()
        .map(|(cols, rows)| WindowSize { rows, cols })
        .unwrap_or(WindowSize { rows: 24, cols: 80 })
}

/// A pty nobody sized yet reports 0x0. The child is told the truth; the
/// screen model needs cells to exist and gets the classic default.
fn model_size(size: WindowSize) -> WindowSize {
    if size.rows == 0 || size.cols == 0 {
        WindowSize { rows: 24, cols: 80 }
    } else {
        size
    }
}

fn pty_size(size: WindowSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn command_builder(command: Command) -> CommandBuilder {
    let mut builder = if let Some(argv0) = command.argv0 {
        let mut builder = CommandBuilder::new("env");
        let mut option = OsString::from("--argv0=");
        option.push(argv0);
        builder.arg(option);
        builder.arg(command.program);
        builder.args(command.args);
        builder
    } else {
        let mut builder = CommandBuilder::new(command.program);
        builder.args(command.args);
        builder
    };
    if let Ok(dir) = std::env::current_dir() {
        builder.cwd(dir);
    }
    builder.env("HEBREW_TTY", "1");
    builder
}

fn child_exit_code(status: WaitStatus) -> Option<i32> {
    status
        .exit_status()
        .or_else(|| status.terminating_signal().map(|signal| 128 + signal))
}

fn terminate_foreground(master: &dyn MasterPty, child: &mut dyn Child, signal: Signal) {
    if let Some(group) = master.process_group_leader().map(Pid::from_raw) {
        let _ = killpg(group, signal);
    } else {
        let _ = child.kill();
    }
}

enum OutputEvent {
    Data(Vec<u8>),
    Done(io::Result<u64>),
}

fn read_output(mut reader: Box<dyn io::Read + Send>, sender: mpsc::SyncSender<OutputEvent>) {
    let mut buffer = [0; 16 * 1024];
    let mut total = 0;
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                let _ = sender.send(OutputEvent::Done(Ok(total)));
                break;
            }
            Ok(read) => {
                total += u64::try_from(read).unwrap_or(0);
                if sender
                    .send(OutputEvent::Data(buffer[..read].to_vec()))
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                let _ = sender.send(OutputEvent::Done(Err(error)));
                break;
            }
        }
    }
}

const FOREGROUND_POLL: Duration = Duration::from_millis(200);

fn adopt_process_name(name: &str) {
    let truncated: String = name.chars().take(15).collect();
    if let Ok(name) = CString::new(truncated) {
        let _ = nix::sys::prctl::set_name(&name);
    }
}

/// Watches which program holds the inner pty and hands each new one to the
/// classifier on a thread, so a slow `--version` never stalls the relay. A
/// verdict comes back tagged with the generation it answers; a stale one is
/// dropped.
struct Follower {
    classifier: Arc<dyn ForegroundClassifier>,
    follow_name: bool,
    own_exe: Option<PathBuf>,
    current: Option<(i32, PathBuf)>,
    generation: u64,
    last_poll: Option<Instant>,
    verdict_tx: mpsc::Sender<(u64, Classified)>,
    verdict_rx: mpsc::Receiver<(u64, Classified)>,
}

impl Follower {
    fn new(classifier: Arc<dyn ForegroundClassifier>, follow_name: bool) -> Self {
        let (verdict_tx, verdict_rx) = mpsc::channel();
        Self {
            classifier,
            follow_name,
            own_exe: std::env::current_exe().ok(),
            current: None,
            generation: 0,
            last_poll: None,
            verdict_tx,
            verdict_rx,
        }
    }

    fn poll<W: RelayWriter>(
        &mut self,
        master: &dyn MasterPty,
        relay: &mut Transform<W>,
        force: bool,
    ) {
        if !force
            && self
                .last_poll
                .is_some_and(|last| last.elapsed() < FOREGROUND_POLL)
        {
            return;
        }
        self.last_poll = Some(Instant::now());
        let Some(group) = master.process_group_leader() else {
            return;
        };
        let Some(exe) = exe_of(group) else {
            return;
        };
        // Before its exec the child is still this binary.
        if self.own_exe.as_deref() == Some(exe.as_path()) {
            return;
        }
        if self.current.as_ref().is_some_and(|(current_group, current_exe)| {
            *current_group == group && *current_exe == exe
        }) {
            return;
        }
        let Some(foreground) = Foreground::read(group) else {
            return;
        };
        self.current = Some((group, exe));
        self.generation += 1;
        relay.mark_generation();
        let generation = self.generation;
        let classifier = Arc::clone(&self.classifier);
        let sender = self.verdict_tx.clone();
        thread::spawn(move || {
            let _ = sender.send((generation, classifier.classify(&foreground)));
        });
    }

    fn apply<W: RelayWriter>(&mut self, relay: &mut Transform<W>) -> io::Result<()> {
        while let Ok((generation, verdict)) = self.verdict_rx.try_recv() {
            if generation != self.generation {
                continue;
            }
            if self.follow_name {
                adopt_process_name(&verdict.name);
            }
            relay.set_path(verdict.path, verdict.mode)?;
        }
        Ok(())
    }
}

pub fn run(launch: Launch) -> Result<i32, Box<dyn Error>> {
    let Launch {
        command,
        path,
        mode,
        classifier,
        follow_name,
    } = launch;
    if let Some(name) = command.argv0.as_deref().and_then(|value| value.to_str()) {
        adopt_process_name(name);
    }
    let resize_pending = Arc::new(AtomicBool::new(false));
    flag::register(SIGWINCH, Arc::clone(&resize_pending))?;
    let mut forwarded = Signals::new([SIGHUP, SIGINT, SIGQUIT, SIGTERM])?;

    let initial_size = terminal_size();
    let pair = native_pty_system().openpty(pty_size(initial_size))?;
    let reader = pair.master.try_clone_reader()?;
    let mut writer = pair.master.take_writer()?;
    let mut child = pair.slave.spawn_command(command_builder(command))?;
    drop(pair.slave);

    let child_pid = child
        .process_id()
        .and_then(|pid| RustixPid::from_raw(pid as i32))
        .ok_or("child process has no process ID")?;
    let _raw_mode = RawModeGuard::enter()?;
    let (writer_done_tx, writer_done_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = io::copy(&mut io::stdin().lock(), &mut writer);
        let _ = writer_done_tx.send((result, writer));
    });

    let (output_tx, output_rx) = mpsc::sync_channel(8);
    thread::spawn(move || read_output(reader, output_tx));
    let stdout = io::stdout();
    let screen = model_size(initial_size);
    let mut relay = Transform::new(
        TraceWriter::new(stdout.lock()),
        screen.rows,
        screen.cols,
        path,
        mode,
    )?;
    let mut follower = Follower::new(classifier, follow_name);
    let mut completed_writer = None;
    let mut output_result = None;
    let mut relay_failed_at = None;
    let mut forwarded_once = false;
    let exit_code = loop {
        if completed_writer.is_none() {
            completed_writer = writer_done_rx.try_recv().ok();
        }
        while let Ok(event) = output_rx.try_recv() {
            match event {
                OutputEvent::Data(bytes) => {
                    follower.poll(pair.master.as_ref(), &mut relay, true);
                    if let Err(error) = relay.feed(&bytes) {
                        terminate_foreground(pair.master.as_ref(), child.as_mut(), Signal::SIGTERM);
                        relay_failed_at.get_or_insert_with(Instant::now);
                        output_result.get_or_insert(Err(error));
                    }
                }
                OutputEvent::Done(result) => {
                    if result
                        .as_ref()
                        .is_err_and(|error| error.raw_os_error() != Some(5))
                    {
                        terminate_foreground(pair.master.as_ref(), child.as_mut(), Signal::SIGTERM);
                        relay_failed_at.get_or_insert_with(std::time::Instant::now);
                    }
                    if output_result.is_none() {
                        output_result = Some(result);
                    }
                }
            }
        }
        follower.poll(pair.master.as_ref(), &mut relay, false);
        if let Err(error) = follower.apply(&mut relay) {
            terminate_foreground(pair.master.as_ref(), child.as_mut(), Signal::SIGTERM);
            relay_failed_at.get_or_insert_with(Instant::now);
            output_result.get_or_insert(Err(error));
        }
        if let Some((_, status)) = waitpid(Some(child_pid), WaitOptions::NOHANG)? {
            if let Some(code) = child_exit_code(status) {
                break code;
            }
        }
        if relay_failed_at.is_some_and(|failed| failed.elapsed() >= Duration::from_millis(500)) {
            terminate_foreground(pair.master.as_ref(), child.as_mut(), Signal::SIGKILL);
        }
        if resize_pending.swap(false, Ordering::Relaxed) {
            let size = terminal_size();
            pair.master.resize(pty_size(size))?;
            let screen = model_size(size);
            relay.resize(screen.rows, screen.cols)?;
        }
        for number in forwarded.pending() {
            if let Some(group) = pair.master.process_group_leader().map(Pid::from_raw) {
                if forwarded_once {
                    let _ = killpg(group, Signal::SIGKILL);
                } else if let Ok(signal) = Signal::try_from(number) {
                    let _ = killpg(group, signal);
                    forwarded_once = true;
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    };

    drop(completed_writer);
    drop(pair.master);
    while output_result.is_none() {
        match output_rx.recv()? {
            OutputEvent::Data(bytes) => {
                if let Err(error) = relay.feed(&bytes) {
                    output_result = Some(Err(error));
                }
            }
            OutputEvent::Done(result) => output_result = Some(result),
        }
    }
    match output_result.unwrap() {
        Ok(_) => {}
        Err(error) if error.raw_os_error() == Some(5) => {}
        Err(error) => return Err(error.into()),
    }

    Ok(exit_code)
}

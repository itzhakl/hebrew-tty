#![forbid(unsafe_code)]

use std::error::Error;
use std::ffi::OsString;
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use rustix::process::{waitpid, Pid as RustixPid, WaitOptions, WaitStatus};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGWINCH};
use signal_hook::flag;

use super::WindowSize;
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

fn pty_size(size: WindowSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn command_builder(command: Command) -> CommandBuilder {
    if let Some(argv0) = command.argv0 {
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
    }
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

fn relay_output(reader: &mut dyn io::Read) -> io::Result<u64> {
    let mut stdout = io::stdout().lock();
    let mut buffer = [0; 16 * 1024];
    let mut total = 0;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        stdout.write_all(&buffer[..read])?;
        stdout.flush()?;
        total += u64::try_from(read).unwrap_or(0);
    }
    Ok(total)
}

pub fn run(command: Command) -> Result<i32, Box<dyn Error>> {
    let resize_pending = Arc::new(AtomicBool::new(false));
    flag::register(SIGWINCH, Arc::clone(&resize_pending))?;
    let forwarded = [SIGHUP, SIGINT, SIGQUIT, SIGTERM]
        .into_iter()
        .map(|number| {
            let pending = Arc::new(AtomicBool::new(false));
            flag::register(number, Arc::clone(&pending))?;
            Ok((number, pending))
        })
        .collect::<Result<Vec<_>, io::Error>>()?;

    let pair = native_pty_system().openpty(pty_size(terminal_size()))?;
    let mut reader = pair.master.try_clone_reader()?;
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

    let (output_done_tx, output_done_rx) = mpsc::channel();
    thread::spawn(move || {
        let result = relay_output(reader.as_mut());
        let _ = output_done_tx.send(result);
    });

    let mut completed_writer = None;
    let mut output_result = None;
    let mut relay_failed_at = None;
    let exit_code = loop {
        if completed_writer.is_none() {
            completed_writer = writer_done_rx.try_recv().ok();
        }
        if let Some((_, status)) = waitpid(Some(child_pid), WaitOptions::NOHANG)? {
            if let Some(code) = child_exit_code(status) {
                break code;
            }
        }
        if output_result.is_none() {
            if let Ok(result) = output_done_rx.try_recv() {
                if result.is_err() {
                    terminate_foreground(pair.master.as_ref(), child.as_mut(), Signal::SIGTERM);
                    relay_failed_at = Some(std::time::Instant::now());
                }
                output_result = Some(result);
            }
        }
        if relay_failed_at.is_some_and(|failed| failed.elapsed() >= Duration::from_millis(500)) {
            terminate_foreground(pair.master.as_ref(), child.as_mut(), Signal::SIGKILL);
        }
        if resize_pending.swap(false, Ordering::Relaxed) {
            pair.master.resize(pty_size(terminal_size()))?;
        }
        for (number, pending) in &forwarded {
            if pending.swap(false, Ordering::Relaxed) {
                if let (Some(group), Ok(signal)) = (
                    pair.master.process_group_leader().map(Pid::from_raw),
                    Signal::try_from(*number),
                ) {
                    let _ = killpg(group, signal);
                }
            }
        }
        thread::sleep(Duration::from_millis(20));
    };

    drop(completed_writer);
    drop(pair.master);
    let output_result = match output_result {
        Some(result) => result,
        None => output_done_rx.recv()?,
    };
    match output_result {
        Ok(_) => {}
        Err(error) if error.raw_os_error() == Some(5) => {}
        Err(error) => return Err(error.into()),
    }

    Ok(exit_code)
}

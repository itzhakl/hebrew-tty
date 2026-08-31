#![forbid(unsafe_code)]

use std::error::Error;
use std::ffi::{CString, OsStr, OsString};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use hebrew_tty::classify::{select_mode, ExecutionPath, RowDisposition};
use hebrew_tty::config::Mode;
use hebrew_tty::layout::is_rtl_char;
use hebrew_tty::render::Renderer;
use hebrew_tty::terminal::TerminalModel;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use rustix::process::{waitpid, Pid as RustixPid, WaitOptions, WaitStatus};
use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGQUIT, SIGTERM, SIGWINCH};
use signal_hook::flag;
use signal_hook::iterator::Signals;

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

#[derive(Clone, Copy, Default)]
enum EscapeState {
    #[default]
    Ground,
    Escape,
    Csi,
    String,
    StringEscape,
}

#[derive(Default)]
struct StreamBoundary {
    escape: EscapeState,
    utf8_continuations: u8,
}

impl StreamBoundary {
    fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.utf8_continuations > 0 {
                if byte & 0b1100_0000 == 0b1000_0000 {
                    self.utf8_continuations -= 1;
                    continue;
                }
                self.utf8_continuations = 0;
            }
            if matches!(byte, 0x18 | 0x1a) {
                self.escape = EscapeState::Ground;
                continue;
            }
            if byte == 0x1b && !matches!(self.escape, EscapeState::String) {
                self.escape = EscapeState::Escape;
                continue;
            }
            self.escape = match self.escape {
                EscapeState::Ground => match byte {
                    0x90 | 0x9d..=0x9f => EscapeState::String,
                    0x9b => EscapeState::Csi,
                    0xc2..=0xdf => {
                        self.utf8_continuations = 1;
                        EscapeState::Ground
                    }
                    0xe0..=0xef => {
                        self.utf8_continuations = 2;
                        EscapeState::Ground
                    }
                    0xf0..=0xf4 => {
                        self.utf8_continuations = 3;
                        EscapeState::Ground
                    }
                    _ => EscapeState::Ground,
                },
                EscapeState::Escape => match byte {
                    b'[' => EscapeState::Csi,
                    b']' | b'P' | b'_' | b'^' => EscapeState::String,
                    0x20..=0x2f => EscapeState::Escape,
                    _ => EscapeState::Ground,
                },
                EscapeState::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        EscapeState::Ground
                    } else {
                        EscapeState::Csi
                    }
                }
                EscapeState::String => match byte {
                    0x07 | 0x9c => EscapeState::Ground,
                    0x1b => EscapeState::StringEscape,
                    _ => EscapeState::String,
                },
                EscapeState::StringEscape => {
                    if byte == b'\\' {
                        EscapeState::Ground
                    } else {
                        EscapeState::String
                    }
                }
            };
        }
    }

    fn is_ground(&self) -> bool {
        matches!(self.escape, EscapeState::Ground) && self.utf8_continuations == 0
    }
}

enum OutputRelay<'a> {
    Passthrough(io::StdoutLock<'a>),
    Transform {
        model: Box<TerminalModel>,
        renderer: Renderer<io::StdoutLock<'a>>,
        path: ExecutionPath,
        mode: Mode,
        corrected: bool,
        boundary: StreamBoundary,
        pending_rows: Vec<u16>,
        pending_cursor: bool,
    },
}

impl<'a> OutputRelay<'a> {
    fn new(
        writer: io::StdoutLock<'a>,
        size: WindowSize,
        path: ExecutionPath,
        mode: Mode,
    ) -> Result<Self, Box<dyn Error>> {
        if select_mode(mode, &path).disposition == RowDisposition::PassThrough {
            return Ok(Self::Passthrough(writer));
        }
        let mut model = TerminalModel::new(size.rows, size.cols)?;
        model.take_dirty_rows();
        Ok(Self::Transform {
            model: Box::new(model),
            renderer: Renderer::new(writer),
            path,
            mode,
            corrected: false,
            boundary: StreamBoundary::default(),
            pending_rows: Vec::new(),
            pending_cursor: false,
        })
    }

    fn feed(&mut self, bytes: &[u8]) -> io::Result<()> {
        match self {
            Self::Passthrough(writer) => {
                writer.write_all(bytes)?;
                writer.flush()
            }
            Self::Transform {
                model,
                renderer,
                path,
                mode,
                corrected,
                boundary,
                pending_rows,
                pending_cursor,
            } => {
                let before = model.cursor();
                if *corrected && boundary.is_ground() {
                    write!(
                        renderer.writer_mut(),
                        "\x1b[{};{}H",
                        before.row + 1,
                        before.col + 1
                    )?;
                }
                renderer.writer_mut().write_all(bytes)?;
                boundary.feed(bytes);
                model.feed(bytes);
                pending_rows.extend(model.take_dirty_rows().into_iter().map(|row| row.row_index));
                let snapshot = model.snapshot();
                *pending_cursor |= before != snapshot.cursor;
                if *corrected {
                    pending_rows.extend(rtl_rows(&snapshot));
                }
                pending_rows.sort_unstable();
                pending_rows.dedup();
                if !boundary.is_ground() {
                    return renderer.writer_mut().flush();
                }
                if !screen_has_rtl(&snapshot) {
                    if *corrected && (!pending_rows.is_empty() || *pending_cursor) {
                        renderer.repaint_dirty(&snapshot, path, *mode, pending_rows)?;
                    }
                    *corrected = false;
                    pending_rows.clear();
                    *pending_cursor = false;
                    return renderer.writer_mut().flush();
                }
                if pending_rows.is_empty() && !*pending_cursor {
                    return renderer.writer_mut().flush();
                }
                renderer.repaint_dirty(&snapshot, path, *mode, pending_rows)?;
                *corrected = true;
                pending_rows.clear();
                *pending_cursor = false;
                Ok(())
            }
        }
    }

    fn resize(&mut self, size: WindowSize) -> Result<(), Box<dyn Error>> {
        if let Self::Transform {
            model,
            renderer,
            path,
            mode,
            corrected,
            pending_rows,
            pending_cursor,
            ..
        } = self
        {
            model.resize(size.rows, size.cols)?;
            let invalidated = model
                .take_dirty_rows()
                .into_iter()
                .map(|row| row.row_index)
                .collect::<Vec<_>>();
            let snapshot = model.snapshot();
            if screen_has_rtl(&snapshot) {
                renderer.repaint_dirty(&snapshot, path, *mode, &invalidated)?;
                *corrected = true;
            } else {
                *corrected = false;
            }
            pending_rows.clear();
            *pending_cursor = false;
        }
        Ok(())
    }
}

fn rtl_rows(screen: &hebrew_tty::terminal::ScreenSnapshot) -> impl Iterator<Item = u16> + '_ {
    screen
        .physical_rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            row.cells
                .iter()
                .any(|cell| cell.text.chars().any(is_rtl_char))
        })
        .map(|(index, _)| index as u16)
}

fn screen_has_rtl(screen: &hebrew_tty::terminal::ScreenSnapshot) -> bool {
    rtl_rows(screen).next().is_some()
}

fn adopt_agent_process_name(argv0: Option<&OsStr>) {
    let Some(name) = argv0.and_then(|value| value.to_str()) else {
        return;
    };
    let truncated: String = name.chars().take(15).collect();
    if let Ok(name) = CString::new(truncated) {
        let _ = nix::sys::prctl::set_name(&name);
    }
}

pub fn run(command: Command, path: ExecutionPath, mode: Mode) -> Result<i32, Box<dyn Error>> {
    adopt_agent_process_name(command.argv0.as_deref());
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
    let mut relay = OutputRelay::new(stdout.lock(), initial_size, path, mode)?;
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
                    if let Err(error) = relay.feed(&bytes) {
                        terminate_foreground(pair.master.as_ref(), child.as_mut(), Signal::SIGTERM);
                        relay_failed_at.get_or_insert_with(std::time::Instant::now);
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
            relay.resize(size)?;
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

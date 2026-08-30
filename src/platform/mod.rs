#![forbid(unsafe_code)]

use std::error::Error;

use hebrew_tty::classify::ExecutionPath;
use hebrew_tty::config::Mode;

use crate::cli::Command;

#[cfg(target_os = "linux")]
mod linux;

#[derive(Clone, Copy)]
pub struct WindowSize {
    pub rows: u16,
    pub cols: u16,
}

#[cfg(target_os = "linux")]
pub fn run(command: Command, path: ExecutionPath, mode: Mode) -> Result<i32, Box<dyn Error>> {
    linux::run(command, path, mode)
}

#[cfg(not(target_os = "linux"))]
compile_error!("hebrew-tty currently supports Linux only");

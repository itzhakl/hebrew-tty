#![forbid(unsafe_code)]

use std::error::Error;
use std::sync::Arc;

use hebrew_tty::classify::ExecutionPath;
use hebrew_tty::config::Mode;

use crate::cli::Command;

pub mod foreground;
#[cfg(target_os = "linux")]
mod linux;

#[derive(Clone, Copy)]
pub struct WindowSize {
    pub rows: u16,
    pub cols: u16,
}

/// The verdict on what the inner pty is running: the name to carry, the
/// execution path, and the mode the policy picked for it.
pub struct Classified {
    pub name: String,
    pub path: ExecutionPath,
    pub mode: Mode,
}

pub trait ForegroundClassifier: Send + Sync {
    fn classify(&self, foreground: &foreground::Foreground) -> Classified;
}

pub struct Launch {
    pub command: Command,
    pub path: ExecutionPath,
    pub mode: Mode,
    pub classifier: Arc<dyn ForegroundClassifier>,
    /// Rename the proxy after whatever comes to the foreground. Off once
    /// `--as` pinned a name.
    pub follow_name: bool,
}

#[cfg(target_os = "linux")]
pub fn run(launch: Launch) -> Result<i32, Box<dyn Error>> {
    linux::run(launch)
}

#[cfg(not(target_os = "linux"))]
compile_error!("hebrew-tty currently supports Linux only");

#![forbid(unsafe_code)]

use std::ffi::OsString;

pub const USAGE: &str = "usage: hebrew-tty [--as NAME] <command> [args...]";
pub const HELP: &str = "usage: hebrew-tty [--as NAME] <command> [args...]\n\n  --as NAME   run the command under that process name; herdr finds an\n              agent pane by it, and a versioned build name is unknown\n";

pub struct Command {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub argv0: Option<OsString>,
}

pub enum Action {
    Help,
    Run(Command),
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action, &'static str> {
    let mut args = args.into_iter();
    let first = args.next().ok_or("missing command")?;
    if first == "--help" {
        return Ok(Action::Help);
    }

    let (argv0, program) = if first == "--as" {
        let name = args.next().ok_or("--as requires a name")?;
        let program = args.next().ok_or("missing command")?;
        (Some(name), program)
    } else {
        (None, first)
    };

    Ok(Action::Run(Command {
        program,
        args: args.collect(),
        argv0,
    }))
}

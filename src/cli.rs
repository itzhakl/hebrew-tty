#![forbid(unsafe_code)]

use std::ffi::OsString;
use std::str::FromStr;

use hebrew_tty::config::Mode;

pub const USAGE: &str = "usage: hebrew-tty [--mode MODE] [--diagnostics PATH] [--as NAME] [<command> [args...]]\n       hebrew-tty --install | --uninstall";
pub const HELP: &str = "usage: hebrew-tty [--mode MODE] [--diagnostics PATH] [--as NAME] [<command> [args...]]\n       hebrew-tty --install | --uninstall\n\n  <command>           what to run under the proxy; without one, $SHELL, and\n                      whatever it brings to the foreground is classified then\n  --mode MODE         auto, logical, visual, or passthrough\n  --diagnostics PATH  append structured JSON diagnostics\n  --as NAME           run the command under that process name; herdr finds an\n                      agent pane by it, and a versioned build name is unknown\n  --install           start every interactive shell under the proxy\n  --uninstall         undo --install\n";

pub struct Command {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub argv0: Option<OsString>,
}

pub struct Invocation {
    pub command: Option<Command>,
    pub mode: Option<Mode>,
    pub diagnostics: Option<OsString>,
}

pub enum Action {
    Help,
    Install,
    Uninstall,
    Run(Invocation),
}

pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action, &'static str> {
    let mut args = args.into_iter();
    let mut argv0 = None;
    let mut mode = None;
    let mut diagnostics = None;

    let program = loop {
        let Some(argument) = args.next() else {
            break None;
        };
        if argument == "--help" {
            return Ok(Action::Help);
        }
        if argument == "--install" {
            return Ok(Action::Install);
        }
        if argument == "--uninstall" {
            return Ok(Action::Uninstall);
        }
        if argument == "--as" {
            argv0 = Some(args.next().ok_or("--as requires a name")?);
        } else if argument == "--mode" {
            let value = args.next().ok_or("--mode requires a value")?;
            let value = value.to_str().ok_or("mode must be valid UTF-8")?;
            mode = Some(Mode::from_str(value)?);
        } else if argument == "--diagnostics" {
            diagnostics = Some(args.next().ok_or("--diagnostics requires a path")?);
        } else if argument == "--" {
            break Some(args.next().ok_or("missing command after --")?);
        } else {
            break Some(argument);
        }
    };

    Ok(Action::Run(Invocation {
        command: program.map(|program| Command {
            program,
            args: args.collect(),
            argv0,
        }),
        mode,
        diagnostics,
    }))
}

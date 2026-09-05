#![forbid(unsafe_code)]

//! What the inner pty is running right now. The pty's foreground process
//! group names the program the user is typing at. The agents ship behind
//! launcher scripts, so the leader can be an interpreter with the agent as
//! its script; both are offered to the classifier, the leader first.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Foreground {
    pub group: i32,
    pub exe: PathBuf,
    pub argv: Vec<OsString>,
}

/// A name the classifier may have a recording for, and the program to ask
/// for the version that verifies it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub name: String,
    pub program: OsString,
}

const INTERPRETERS: &[&str] = &[
    "node", "nodejs", "bun", "deno", "python", "python3", "sh", "bash", "zsh", "dash",
];

/// The executable behind a pid, with the marker the kernel appends once the
/// file has been replaced underneath it.
pub fn exe_of(pid: i32) -> Option<PathBuf> {
    let target = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let bytes = target.as_os_str().as_bytes();
    let trimmed = bytes.strip_suffix(b" (deleted)").unwrap_or(bytes);
    Some(PathBuf::from(OsStr::from_bytes(trimmed)))
}

impl Foreground {
    pub fn read(group: i32) -> Option<Self> {
        let exe = exe_of(group)?;
        let cmdline = fs::read(format!("/proc/{group}/cmdline")).ok()?;
        let argv = cmdline
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| OsStr::from_bytes(part).to_owned())
            .collect();
        Some(Self { group, exe, argv })
    }

    pub fn candidates(&self) -> Vec<Candidate> {
        candidates(&self.exe, &self.argv)
    }

    /// What to call the proxy while this runs and nothing recorded matches.
    pub fn display_name(&self) -> String {
        self.candidates()
            .into_iter()
            .next()
            .map(|candidate| candidate.name)
            .unwrap_or_else(|| "hebrew-tty".to_owned())
    }
}

pub fn candidates(exe: &Path, argv: &[OsString]) -> Vec<Candidate> {
    let mut found = Vec::new();
    let Some(name) = argv
        .first()
        .and_then(|argv0| file_name(argv0))
        .or_else(|| file_name(exe.as_os_str()))
    else {
        return found;
    };
    let interpreter = INTERPRETERS.contains(&name.as_str());
    found.push(Candidate {
        name,
        program: exe.as_os_str().to_owned(),
    });
    if interpreter {
        let script = argv
            .iter()
            .skip(1)
            .find(|argument| !argument.as_bytes().starts_with(b"-"));
        if let Some(name) = script.and_then(|script| file_stem(script)) {
            found.push(Candidate {
                program: OsString::from(&name),
                name,
            });
        }
    }
    found
}

fn file_name(value: &OsStr) -> Option<String> {
    let name = Path::new(value).file_name()?.to_str()?;
    Some(name.trim_start_matches('-').to_owned())
}

fn file_stem(value: &OsStr) -> Option<String> {
    Path::new(value).file_stem()?.to_str().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<OsString> {
        items.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_native_agent_is_named_by_argv0_and_probed_by_its_exe() {
        let exe = Path::new("/home/me/.local/share/claude/versions/2.1.261");
        let found = candidates(exe, &argv(&["claude", "--resume"]));
        assert_eq!(
            found,
            vec![Candidate {
                name: "claude".to_owned(),
                program: OsString::from(exe),
            }]
        );
    }

    #[test]
    fn an_interpreter_offers_its_script_as_a_second_candidate() {
        let exe = Path::new("/usr/bin/node");
        let found = candidates(
            exe,
            &argv(&["node", "--no-warnings", "/opt/pnpm/@openai/codex/bin/codex.js"]),
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "node");
        assert_eq!(
            found[1],
            Candidate {
                name: "codex".to_owned(),
                program: OsString::from("codex"),
            }
        );
    }

    #[test]
    fn a_launcher_script_names_the_agent_before_it_execs() {
        let exe = Path::new("/usr/bin/bash");
        let found = candidates(exe, &argv(&["/bin/bash", "/home/me/.local/bin/pi"]));
        assert_eq!(found[1].name, "pi");
    }

    #[test]
    fn a_login_shell_drops_its_dash_and_an_empty_cmdline_falls_back_to_the_exe() {
        assert_eq!(candidates(Path::new("/bin/zsh"), &argv(&["-zsh"]))[0].name, "zsh");
        assert_eq!(candidates(Path::new("/bin/zsh"), &[])[0].name, "zsh");
        assert!(candidates(Path::new(""), &[]).is_empty());
    }
}

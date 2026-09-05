//! `hebrew-tty` with no command wraps the shell, and `--install` makes every
//! interactive shell do that on its own.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_hebrew-tty")
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hebrew-tty-shell-wrap-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A PATH without any `hebrew-tty` on it, so the block names the binary under
/// test rather than whatever launcher the machine has installed.
const BARE_PATH: &str = "/usr/bin:/bin";

fn with_home(home: &Path, shell: &str, args: &[&str]) -> Output {
    Command::new(binary())
        .args(args)
        .env("HOME", home)
        .env("SHELL", shell)
        .env("PATH", BARE_PATH)
        .env_remove("ZDOTDIR")
        .env_remove("HEBREW_TTY")
        .output()
        .unwrap()
}

#[test]
fn without_a_command_the_shell_runs_under_the_proxy_and_is_marked() {
    let dir = temp_dir("shell");
    let shell = dir.join("shell");
    executable(&shell, "#!/bin/sh\nprintf '%s' \"$HEBREW_TTY\"\nexit 7\n");

    let output = Command::new(binary())
        .env("SHELL", &shell)
        .env_remove("HEBREW_TTY")
        .output()
        .unwrap();

    assert_eq!(output.stdout, b"1");
    assert_eq!(output.status.code(), Some(7));
}

#[test]
fn an_agent_started_from_the_wrapped_shell_is_classified_and_repaired() {
    let dir = temp_dir("agent");
    executable(
        &dir.join("claude"),
        "#!/bin/sh\ncase \"$1\" in --version) echo '2.1.261 (Claude Code)'; exit 0;; esac\nprintf 'שלום עולם'\nsleep 0.5\n",
    );
    let diagnostics = dir.join("diagnostics.jsonl");

    let output = Command::new(binary())
        .args([
            "--diagnostics",
            diagnostics.to_str().unwrap(),
            "bash",
            "-ic",
            "claude; exit",
        ])
        .env("PATH", format!("{}:{BARE_PATH}", dir.display()))
        .env("HOME", &dir)
        .env("TERM", "xterm")
        .env_remove("HEBREW_TTY")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");

    let records = fs::read_to_string(&diagnostics).unwrap();
    let mut lines = records.lines();
    assert!(
        lines.next().unwrap().contains("\"command\":\"bash\""),
        "the launch verdict comes first: {records}"
    );
    let claude = lines
        .find(|line| line.contains("\"command\":\"claude\""))
        .unwrap_or_else(|| panic!("no verdict for the agent: {records}"));
    assert!(claude.contains("\"confidence\":\"verified\""), "{claude}");
    assert!(claude.contains("\"row_disposition\":\"recover_visual\""), "{claude}");

    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.matches('ש').count() >= 2,
        "the row is forwarded once and repainted once: {text:?}"
    );
    assert!(
        text.contains(&format!("{}שלום", " ".repeat(40))),
        "the repaint flushes the row to the right edge: {text:?}"
    );
}

#[test]
fn install_and_uninstall_edit_the_rc_file_once() {
    let home = temp_dir("install");
    let rc = home.join(".zshrc");
    fs::write(&rc, "export PATH=~/bin:$PATH\n").unwrap();

    let installed = with_home(&home, "/bin/zsh", &["--install"]);
    assert!(installed.status.success(), "{installed:?}");
    assert!(String::from_utf8(installed.stdout)
        .unwrap()
        .contains(".zshrc"));
    let text = fs::read_to_string(&rc).unwrap();
    assert!(text.starts_with("# >>> hebrew-tty >>>\n"), "{text}");
    assert!(text.contains("exec '"), "{text}");
    assert!(
        text.ends_with("# <<< hebrew-tty <<<\n\nexport PATH=~/bin:$PATH\n"),
        "{text}"
    );

    let again = with_home(&home, "/bin/zsh", &["--install"]);
    assert!(again.status.success());
    assert_eq!(fs::read_to_string(&rc).unwrap(), text);

    let removed = with_home(&home, "/bin/zsh", &["--uninstall"]);
    assert!(removed.status.success());
    assert_eq!(
        fs::read_to_string(&rc).unwrap(),
        "export PATH=~/bin:$PATH\n"
    );

    let unsupported = with_home(&home, "/usr/bin/fish", &["--install"]);
    assert_eq!(unsupported.status.code(), Some(1));
    assert!(String::from_utf8(unsupported.stderr)
        .unwrap()
        .contains("fish"));
}

#[test]
fn the_installed_block_wraps_an_interactive_shell_exactly_once() {
    let home = temp_dir("block");
    let rc = home.join(".bashrc");
    fs::write(&rc, "printf 'rc-ran:%s\\n' \"${HEBREW_TTY:-unset}\"\n").unwrap();
    let inner = home.join("inner-shell");
    executable(
        &inner,
        "#!/bin/sh\nexec bash --rcfile \"$HOME/.bashrc\" -i -c true\n",
    );

    let installed = with_home(&home, "/bin/bash", &["--install"]);
    assert!(installed.status.success(), "{installed:?}");
    let text = fs::read_to_string(&rc).unwrap();
    let named = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("case $- in *i*) exec '"))
        .and_then(|rest| rest.split('\'').next())
        .unwrap();
    assert_eq!(
        fs::canonicalize(named).unwrap(),
        fs::canonicalize(binary()).unwrap(),
        "the block names the binary that installed it"
    );

    // The proxy under test is also the pty provider for the outer shell; `env -u`
    // takes back the marker it sets, so the outer shell is a fresh terminal.
    let session = Command::new("timeout")
        .args(["20", binary(), "env", "-u", "HEBREW_TTY", "bash", "--rcfile"])
        .arg(&rc)
        .args(["-i", "-c", "true"])
        .env("HOME", &home)
        .env("SHELL", &inner)
        .env("PATH", BARE_PATH)
        .env("TERM", "xterm")
        .env_remove("HEBREW_TTY")
        .output()
        .unwrap();
    let transcript = String::from_utf8_lossy(&session.stdout);
    assert_eq!(
        transcript.matches("rc-ran:").count(),
        1,
        "the outer shell leaves before its rc finishes; the inner runs it once: {transcript}"
    );
    assert!(transcript.contains("rc-ran:1"), "{transcript}");
    assert!(session.status.success(), "{session:?}");
}

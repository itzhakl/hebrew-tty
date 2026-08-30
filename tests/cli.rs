#![forbid(unsafe_code)]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_hebrew-tty")
}

fn run(args: &[&str]) -> Output {
    Command::new(binary()).args(args).output().unwrap()
}

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("hebrew-tty-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}

fn python_probe(script: &str, args: &[&str]) -> Output {
    let mut command = Command::new("python3");
    command
        .arg("-c")
        .arg(script)
        .arg(binary())
        .args(args)
        .stdin(Stdio::null());
    command.output().unwrap()
}

#[test]
fn help_and_errors_have_stable_statuses() {
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert_eq!(
        String::from_utf8(help.stdout).unwrap(),
        "usage: hebrew-tty [--as NAME] <command> [args...]\n\n  --as NAME   run the command under that process name; herdr finds an\n              agent pane by it, and a versioned build name is unknown\n"
    );

    let missing = run(&[]);
    assert_eq!(missing.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(missing.stderr).unwrap(),
        "hebrew-tty: missing command\nusage: hebrew-tty [--as NAME] <command> [args...]\n"
    );

    let missing_name = run(&["--as"]);
    assert_eq!(missing_name.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(missing_name.stderr).unwrap(),
        "hebrew-tty: --as requires a name\nusage: hebrew-tty [--as NAME] <command> [args...]\n"
    );
}

#[test]
fn passthrough_preserves_bytes_and_exit_status() {
    let output = run(&["sh", "-c", "printf '\\001plain \\377\\n'; exit 37"]);
    assert_eq!(output.stdout, b"\x01plain \xff\r\n");
    assert_eq!(output.status.code(), Some(37));
}

#[test]
fn passthrough_relays_stdin_bytes_exactly() {
    let mut child = Command::new(binary())
        .args([
            "sh",
            "-c",
            "stty raw -echo; printf ready; dd bs=1 count=5 2>/dev/null",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut ready = [0; 5];
    stdout.read_exact(&mut ready).unwrap();
    assert_eq!(&ready, b"ready");
    let mut input = child.stdin.take().unwrap();
    input.write_all(b"\0\xffAZ\n").unwrap();
    input.flush().unwrap();
    drop(input);
    let mut bytes = [0; 5];
    stdout.read_exact(&mut bytes).unwrap();
    let status = child.wait().unwrap();

    assert!(status.success());
    assert_eq!(&bytes, b"\0\xffAZ\n");
}

#[test]
fn passthrough_does_not_append_eof_bytes_in_raw_mode() {
    let mut child = Command::new(binary())
        .args([
            "sh",
            "-c",
            "stty raw -echo; printf ready; exec python3 -c 'import os, select, sys; data = os.read(0, 3); readable = select.select([0], [], [], 0.3)[0]; extra = os.read(0, 16) if readable else b\"\"; sys.stdout.write((data + extra).hex())'",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut ready = [0; 5];
    stdout.read_exact(&mut ready).unwrap();
    assert_eq!(&ready, b"ready");
    child.stdin.take().unwrap().write_all(b"abc").unwrap();

    let mut output = Vec::new();
    stdout.read_to_end(&mut output).unwrap();
    let status = child.wait().unwrap();

    assert!(status.success());
    assert_eq!(output, b"616263");
}

#[test]
fn passthrough_maps_child_signal_to_shell_status() {
    let terminated = run(&["sh", "-c", "kill -TERM $$"]);
    assert_eq!(terminated.status.code(), Some(143));

    let killed = run(&["sh", "-c", "kill -KILL $$"]);
    assert_eq!(killed.status.code(), Some(137));

    let realtime = run(&[
        "python3",
        "-c",
        "import os, signal; os.kill(os.getpid(), signal.SIGRTMIN)",
    ]);
    assert_eq!(realtime.status.code(), Some(162));
}

#[test]
fn passthrough_signals_the_current_foreground_process_group() {
    const DRIVER: &str = r#"
import os, pty, select, signal, sys, time
pid, fd = pty.fork()
if pid == 0:
    command = """
import os, signal, sys
read_fd, write_fd = os.pipe()
child = os.fork()
if child == 0:
    os.close(write_fd)
    os.setpgid(0, 0)
    os.read(read_fd, 1)
    def interrupted(number, frame):
        print('JOB_INT', flush=True)
        os._exit(42)
    signal.signal(signal.SIGINT, interrupted)
    print('READY', flush=True)
    signal.pause()
os.close(read_fd)
signal.signal(signal.SIGINT, signal.SIG_IGN)
signal.signal(signal.SIGTTOU, signal.SIG_IGN)
os.setpgid(child, child)
os.tcsetpgrp(0, child)
os.write(write_fd, b'x')
os.close(write_fd)
_, status = os.waitpid(child, 0)
os.tcsetpgrp(0, os.getpgrp())
sys.exit(os.waitstatus_to_exitcode(status))
"""
    os.execv(sys.argv[1], [sys.argv[1], 'python3', '-c', command])
data = b''
deadline = time.time() + 5
while b'READY' not in data and time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], .1)
    if ready: data += os.read(fd, 4096)
if b'READY' not in data:
    os.kill(pid, signal.SIGKILL)
    raise SystemExit('foreground job did not start')
os.kill(pid, signal.SIGINT)
while b'JOB_INT' not in data and time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], .1)
    if ready:
        try: data += os.read(fd, 4096)
        except OSError: break
if b'JOB_INT' not in data:
    os.kill(pid, signal.SIGKILL)
_, status = os.waitpid(pid, 0)
sys.stdout.buffer.write(data)
if b'JOB_INT' not in data:
    raise SystemExit('foreground job did not receive SIGINT')
"#;
    let output = python_probe(DRIVER, &[]);
    assert!(
        output.status.success(),
        "stdout={:?} stderr={}",
        output.stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.windows(7).any(|part| part == b"JOB_INT"));
}

#[test]
fn passthrough_stops_when_stdout_consumer_closes() {
    const DRIVER: &str = r#"
import subprocess, sys
proc = subprocess.Popen([sys.argv[1], 'sh', '-c', 'yes x'], stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
assert proc.stdout.readline() == b'x\r\n'
proc.stdout.close()
try:
    proc.wait(timeout=3)
except subprocess.TimeoutExpired:
    proc.kill()
    proc.wait()
    raise SystemExit('proxy did not stop after its stdout closed')
"#;
    let output = python_probe(DRIVER, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn passthrough_drains_output_after_child_exit() {
    let mut child = Command::new(binary())
        .args([
            "sh",
            "-c",
            "stty raw -echo; exec python3 -c \"import os; os.write(1, b'x' * 65536)\"",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    thread::sleep(Duration::from_millis(700));

    let mut output = Vec::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_end(&mut output)
        .unwrap();
    let status = child.wait().unwrap();

    assert!(status.success());
    assert_eq!(output.len(), 65_536);
    assert!(output.iter().all(|byte| *byte == b'x'));
}

#[test]
fn passthrough_child_has_controlling_tty_and_initial_dimensions() {
    let output = run(&[
        "sh",
        "-c",
        "test -t 0 && test -t 1 && test -t 2 && stty size",
    ]);
    assert!(
        output.status.success(),
        "stdout={:?} stderr={}",
        output.stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"24 80\r\n");
}

#[test]
fn passthrough_propagates_resize_from_parent_pty() {
    const DRIVER: &str = r#"
import fcntl, os, pty, select, signal, struct, sys, termios, time
pid, fd = pty.fork()
if pid == 0:
    os.execv(sys.argv[1], [sys.argv[1], 'sh', '-c', "trap 'stty size; exit' WINCH; echo ready; while :; do sleep 1; done"])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 31, 97, 0, 0))
data = b''
deadline = time.time() + 5
while b'ready' not in data and time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], .1)
    if ready: data += os.read(fd, 4096)
time.sleep(.2)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack('HHHH', 43, 109, 0, 0))
os.kill(pid, signal.SIGWINCH)
while b'43 109' not in data and time.time() < deadline:
    ready, _, _ = select.select([fd], [], [], .1)
    if ready: data += os.read(fd, 4096)
if b'43 109' not in data:
    os.kill(pid, signal.SIGKILL)
_, status = os.waitpid(pid, 0)
sys.stdout.buffer.write(data)
sys.exit(os.waitstatus_to_exitcode(status))
"#;
    let output = python_probe(DRIVER, &[]);
    assert!(
        output.status.success(),
        "stdout={:?} stderr={}",
        output.stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.windows(6).any(|part| part == b"43 109"));
}

#[test]
fn as_overrides_child_argv0() {
    const DRIVER: &str = r#"
import os, pty, sys, time
pid, fd = pty.fork()
if pid == 0:
    os.execv(sys.argv[1], [sys.argv[1], '--as', 'claude', 'sleep', '5'])
deadline = time.time() + 5
child = None
while time.time() < deadline:
    children = open(f'/proc/{pid}/task/{pid}/children').read().split()
    if children:
        child = children[0]
        break
    time.sleep(.02)
if child is None:
    os.kill(pid, 9)
    raise SystemExit('child process did not start')
cmdline = open(f'/proc/{child}/cmdline', 'rb').read().split(b'\0')[0]
os.kill(pid, 15)
os.waitpid(pid, 0)
sys.stdout.buffer.write(cmdline)
"#;
    let output = python_probe(DRIVER, &[]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"claude");
}

#[test]
fn compatibility_launcher_resolves_explicit_binary_without_recursion() {
    let dir = temp_dir("launcher");
    let fake = dir.join("rust-binary");
    fs::write(&fake, "#!/bin/sh\nprintf '%s\\n' \"$*\"\n").unwrap();
    let mut permissions = fs::metadata(&fake).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake, permissions).unwrap();

    let launcher = Path::new(env!("CARGO_MANIFEST_DIR")).join("bin/hebrew-tty");
    let output = Command::new(launcher)
        .env("HEBREW_TTY_BIN", &fake)
        .args(["one", "two words"])
        .output()
        .unwrap();
    fs::remove_dir_all(dir).unwrap();

    assert!(output.status.success());
    assert_eq!(output.stdout, b"one two words\n");
}

#[test]
fn compatibility_launcher_rejects_explicit_self_reference() {
    let launcher = Path::new(env!("CARGO_MANIFEST_DIR")).join("bin/hebrew-tty");
    let output = Command::new(&launcher)
        .env("HEBREW_TTY_BIN", &launcher)
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(127));
    assert_eq!(
        output.stderr,
        b"hebrew-tty: Rust binary not found; run cargo build --release\n"
    );
}

#[test]
fn compatibility_launcher_rejects_itself_on_path() {
    let dir = temp_dir("launcher-recursion");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("bin/hebrew-tty");
    let launcher = dir.join("hebrew-tty");
    symlink(source, &launcher).unwrap();

    let output = Command::new(&launcher)
        .env_remove("HEBREW_TTY_BIN")
        .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
        .output()
        .unwrap();
    fs::remove_dir_all(dir).unwrap();

    assert_eq!(output.status.code(), Some(127));
    assert_eq!(
        output.stderr,
        b"hebrew-tty: Rust binary not found; run cargo build --release\n"
    );
}

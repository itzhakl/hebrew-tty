#![forbid(unsafe_code)]

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        "usage: hebrew-tty [--mode MODE] [--diagnostics PATH] [--as NAME] <command> [args...]\n\n  --mode MODE         auto, logical, visual, or passthrough\n  --diagnostics PATH  append structured JSON diagnostics\n  --as NAME           run the command under that process name; herdr finds an\n                      agent pane by it, and a versioned build name is unknown\n"
    );

    let missing = run(&[]);
    assert_eq!(missing.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(missing.stderr).unwrap(),
        "hebrew-tty: missing command\nusage: hebrew-tty [--mode MODE] [--diagnostics PATH] [--as NAME] <command> [args...]\n"
    );

    let missing_name = run(&["--as"]);
    assert_eq!(missing_name.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(missing_name.stderr).unwrap(),
        "hebrew-tty: --as requires a name\nusage: hebrew-tty [--mode MODE] [--diagnostics PATH] [--as NAME] <command> [args...]\n"
    );

    let invalid_mode = run(&["--mode", "guess", "true"]);
    assert_eq!(invalid_mode.status.code(), Some(2));
    assert!(String::from_utf8(invalid_mode.stderr)
        .unwrap()
        .contains("mode must be auto, logical, visual, or passthrough"));
}

#[test]
fn passthrough_preserves_bytes_and_exit_status() {
    let output = run(&["sh", "-c", "printf '\\001plain \\377\\n'; exit 37"]);
    assert_eq!(output.stdout, b"\x01plain \xff\r\n");
    assert_eq!(output.status.code(), Some(37));
}

#[test]
fn mode_and_diagnostics_options_apply_before_launch() {
    let dir = temp_dir("diagnostics");
    let diagnostics = dir.join("events.jsonl");
    let output = Command::new(binary())
        .args(["--mode", "passthrough", "--diagnostics"])
        .arg(&diagnostics)
        .args(["sh", "-c", "printf ok"])
        .output()
        .unwrap();
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(&diagnostics).unwrap()).unwrap();
    fs::remove_dir_all(dir).unwrap();

    assert!(output.status.success());
    assert_eq!(output.stdout, b"ok");
    assert_eq!(record["selected_mode"], "passthrough");
    assert_eq!(record["row_disposition"], "pass_through");
}

#[test]
fn auto_mode_requires_the_measured_agent_version() {
    let dir = temp_dir("agent-version");
    let agent = dir.join("claude");
    let diagnostics = dir.join("events.jsonl");
    let descendant_pid = dir.join("descendant.pid");
    fs::write(
        &agent,
        "#!/bin/sh\nif [ \"${1:-}\" = --version ]; then sleep 30 & echo $! > \"$DESCENDANT_PID\"; echo '2.1.251 (Claude Code)'; else printf launched; fi\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();

    let started = Instant::now();
    let output = Command::new(binary())
        .args(["--diagnostics"])
        .arg(&diagnostics)
        .arg(&agent)
        .env("DESCENDANT_PID", &descendant_pid)
        .output()
        .unwrap();
    let elapsed = started.elapsed();
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(&diagnostics).unwrap()).unwrap();
    let descendant = fs::read_to_string(&descendant_pid).unwrap();
    let descendant_proc = PathBuf::from(format!("/proc/{}", descendant.trim()));
    for _ in 0..100 {
        if !descendant_proc.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    fs::remove_dir_all(dir).unwrap();

    assert!(output.status.success());
    assert_eq!(output.stdout, b"launched");
    assert!(elapsed < Duration::from_secs(3));
    assert!(
        !descendant_proc.exists(),
        "version-probe descendant survived"
    );
    assert_eq!(record["confidence"], "verified");
    assert_eq!(record["row_disposition"], "recover_visual");
}

#[test]
fn version_probe_bounds_output_from_detached_descendants() {
    let dir = temp_dir("detached-version-probe");
    let agent = dir.join("claude");
    let diagnostics = dir.join("events.jsonl");
    let descendant_pid = dir.join("detached.pid");
    fs::write(
        &agent,
        "#!/bin/sh\nif [ \"${1:-}\" = --version ]; then setsid yes x & echo $! > \"$DETACHED_PID\"; sleep 0.1; echo '2.1.251 (Claude Code)'; else printf launched; fi\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&agent).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&agent, permissions).unwrap();

    let started = Instant::now();
    let output = Command::new(binary())
        .args(["--diagnostics"])
        .arg(&diagnostics)
        .arg(&agent)
        .env("DETACHED_PID", &descendant_pid)
        .output()
        .unwrap();
    let elapsed = started.elapsed();
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(&diagnostics).unwrap()).unwrap();
    let descendant = fs::read_to_string(&descendant_pid).unwrap();
    let descendant_proc = PathBuf::from(format!("/proc/{}", descendant.trim()));
    for _ in 0..100 {
        if !descendant_proc.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    fs::remove_dir_all(dir).unwrap();

    assert!(output.status.success());
    assert_eq!(output.stdout, b"launched");
    assert!(elapsed < Duration::from_secs(3));
    assert!(!descendant_proc.exists(), "detached writer survived");
    assert_eq!(record["confidence"], "unknown");
}

#[test]
fn diagnostics_reject_terminal_stream_aliases() {
    for path in [
        "/dev/stdout",
        "/dev/stderr",
        "/proc/self/fd/1",
        "/proc/self/fd/2",
    ] {
        let output = Command::new(binary())
            .args(["--diagnostics", path, "sh", "-c", "printf launched"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{path}");
        assert!(output.stdout.is_empty(), "{path}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("diagnostics path must"), "{path}: {stderr}");
        assert!(!stderr.contains("\"command\""), "{path}: {stderr}");
    }
}

#[test]
fn diagnostics_reject_a_fifo_without_blocking() {
    let dir = temp_dir("diagnostics-fifo");
    let path = dir.join("events.fifo");
    assert!(Command::new("mkfifo")
        .arg(&path)
        .status()
        .unwrap()
        .success());

    let started = Instant::now();
    let output = Command::new(binary())
        .args(["--diagnostics"])
        .arg(&path)
        .arg("true")
        .output()
        .unwrap();
    let elapsed = started.elapsed();
    fs::remove_dir_all(dir).unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(elapsed < Duration::from_secs(1));
    assert!(output.stdout.is_empty());
}

#[test]
fn diagnostics_reject_a_regular_file_used_for_terminal_output() {
    let dir = temp_dir("diagnostics-output-alias");
    let path = dir.join("combined.log");
    let output_file = fs::File::create(&path).unwrap();
    let output = Command::new(binary())
        .args(["--diagnostics"])
        .arg(&path)
        .args(["sh", "-c", "printf launched"])
        .stdout(output_file)
        .output()
        .unwrap();
    let contents = fs::read(&path).unwrap();
    fs::remove_dir_all(dir).unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(contents.is_empty());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("diagnostics path must not alias stdout or stderr"));
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

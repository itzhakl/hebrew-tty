import fcntl, os, pty, re, select, struct, sys, termios, time

BIN = "/home/itzhakl/Projects/hebrew-tty/target/release/hebrew-tty"
CLAUDE = "/home/itzhakl/.local/share/claude/versions/2.1.252"
COLS, ROWS = 100, 30

mode = sys.argv[1]          # passthrough | auto
text = "שלום עולם"

pid, fd = pty.fork()
if pid == 0:
    os.chdir("/home/itzhakl/Projects/hebrew-tty")
    os.environ["WT_SESSION"] = "herdr"
    os.environ["TERM"] = "xterm-256color"
    args = [BIN]
    if mode != "auto":
        args += ["--mode", mode]
    args += ["--as", "claude", CLAUDE]
    os.execv(BIN, args)

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))


def drain(seconds):
    buf = b""
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.2)
        if not r:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        buf += chunk
    return buf


first = drain(8)
open("boot.txt","w").write(first.decode("utf-8","replace"))
print("BOOT bytes:", len(first))
for ch in text:                # type Hebrew, never press Enter
    os.write(fd, ch.encode())
    time.sleep(0.12)
tail = drain(2.5)

os.kill(pid, 15)
try:
    os.waitpid(pid, 0)
except ChildProcessError:
    pass

out = tail.decode("utf-8", "replace")
path = f"/tmp/claude-1000/-home-itzhakl/9d4a4224-576a-4150-8d5e-c46553ac5c0e/scratchpad/live-{mode}.txt"
open(path, "w").write(out)

print(f"=== mode={mode} cols={COLS} bytes={len(tail)}")
for m in re.finditer(r"[^\x1b]*[֐-׿][^\x1b]*", out):
    print("HEBREW RUN:", repr(m.group()[:80]))
moves = re.findall(r"\x1b\[(?:(\d+);)?(\d+)([HG])", out)
print("last 6 caret moves:", moves[-6:])

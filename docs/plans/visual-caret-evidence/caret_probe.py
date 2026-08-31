import fcntl, os, pty, re, select, struct, sys, termios, time

BIN = "/home/itzhakl/Projects/hebrew-tty/target/release/hebrew-tty"
COLS, ROWS = 60, 10

# Paint Hebrew on row 1 and leave the caret just past its last glyph.
PAYLOAD = "printf '\\033[1;1Hםולש םלוע\\033[1;10H'; sleep 1"

pid, fd = pty.fork()
if pid == 0:
    os.environ["WT_SESSION"] = "herdr"
    os.execv(BIN, [BIN, "--mode", "visual", "--as", "claude", "sh", "-c", PAYLOAD])

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))

out = b""
deadline = time.time() + 3
while time.time() < deadline:
    r, _, _ = select.select([fd], [], [], 0.3)
    if not r:
        continue
    try:
        chunk = os.read(fd, 65536)
    except OSError:
        break
    if not chunk:
        break
    out += chunk

text = out.decode("utf-8", "replace")
print("RAW:", repr(text))

cups = re.findall(r"\x1b\[(\d+);(\d+)H", text)
print("CUP sequences (row, col):", cups)
if cups:
    print("final caret column:", cups[-1][1])

for line in text.split("\r\n"):
    if any("֐" <= ch <= "׿" for ch in line):
        stripped = re.sub(r"\x1b\[[0-9;?]*[a-zA-Z]", "", line)
        lead = len(stripped) - len(stripped.lstrip(" "))
        print(f"text starts at column {lead + 1} of {COLS}")

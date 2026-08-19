#!/usr/bin/env python3
"""Check that Hebrew renders correctly everywhere Claude Code and the shell run.

Reads the answer off the wire instead of off the screen. Each case is run on a
real pty; a Hebrew word is typed in; the bytes painted back are inspected.

  visual order  -> Claude reordered the text itself, so the terminal must not
                   reorder again: VTE has to be in BDSM explicit (ESC[8l).
  logical order -> the writer left the text alone, so VTE has to reorder it:
                   bidi stays on (ESC[8h).

Claude does the first when it paints straight to the terminal and the second
inside tmux, which is why no single setting is right everywhere.

    python3 tools/bidi-probe.py          # the whole matrix
    python3 tools/bidi-probe.py -v       # also dump what each case painted

Exits non-zero if any case would render reversed.
"""
import fcntl
import os
import pty
import select
import struct
import sys
import termios
import time

WORD = "שלום"
OFF = "\x1b[8l"      # BDSM explicit - VTE stops reordering
ON = "\x1b[8h"       # BDSM implicit - VTE reorders
AUTO = "\x1b[?2501h"  # per-line paragraph direction autodetection

TMUX = ["tmux", "-f", "/dev/null", "new-session", "-A", "-s", "bidiprobe"]


def capture(argv, feed, seconds, cols=100, rows=30):
    """Run argv on a pty, type feed a few seconds in, return everything painted."""
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["COLUMNS"], os.environ["LINES"] = str(cols), str(rows)
        os.environ.pop("TMUX", None)
        try:
            os.execvp(argv[0], argv)
        except Exception:
            os._exit(127)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    out, start = bytearray(), time.time()
    pending = list(feed)
    while time.time() - start < seconds:
        readable, _, _ = select.select([fd], [], [], 0.3)
        if readable:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                break
            if not chunk:
                break
            out += chunk
        if pending and time.time() - start > pending[0][0]:
            _, keys = pending.pop(0)
            try:
                os.write(fd, keys)
            except OSError:
                break
    try:
        os.kill(pid, 9)
        os.waitpid(pid, 0)
    except Exception:
        pass
    os.close(fd)
    return bytes(out)


CASES = []


def case(name, argv, feed, seconds, **expect):
    CASES.append((name, argv, feed, seconds, expect))


# --- Claude Code painting straight to the terminal: it reorders, VTE must not.
case("claude, no tmux", ["claude"], [(5, WORD.encode())], 18,
     visual=True, off=True)
case("claude default mode", ["claude", "--settings", '{"tui":"default"}'],
     [(5, WORD.encode())], 18, visual=True, off=True)

# --- Claude Code inside tmux: it emits logical order, VTE must keep reordering.
case("claude in tmux", TMUX + ["claude"], [(6, WORD.encode())], 20,
     logical=True, off=False)
case("claude default in tmux", TMUX + ["claude", "--settings", '{"tui":"default"}'],
     [(6, WORD.encode())], 20, logical=True, off=False)

# --- Through the ~/.bashrc wrapper, the way it is actually typed.
case("wrapper: claude", ["bash", "-i"],
     [(2, b"claude\n"), (9, WORD.encode())], 26, visual=True, off=True)

# --- The shell itself: logical order, bidi on, autodetect on.
case("interactive shell", ["bash", "-i"],
     [(2, "printf 'שלום\\n'\n".encode())], 7,
     logical=True, on=True, auto=True, off=False)
# Inside tmux the shell writes logical order and nothing turns VTE's bidi off,
# so the outer terminal reorders it with its default. .bashrc's autodetect line
# does not fire there - TERM is screen*, and tmux would swallow the escape.
case("shell inside tmux", TMUX + ["bash", "-i"],
     [(3, "printf 'שלום\\n'\n".encode())], 9,
     logical=True, off=False)


def run(name, argv, feed, seconds, expect, verbose):
    painted = capture(argv, feed, seconds).decode("utf8", "replace")
    got = {
        "visual": WORD[::-1] in painted,
        "logical": WORD in painted,
        "off": OFF in painted,
        "on": ON in painted,
        "auto": AUTO in painted,
    }
    bad = [k for k, want in expect.items() if got[k] != want]
    flags = " ".join(f"{k}={str(got[k])[0]}" for k in ("visual", "logical", "off", "on", "auto"))
    print(f"  {name:24} {flags}  {'PASS' if not bad else 'FAIL'}")
    if bad:
        for k in bad:
            print(f"      {k}: expected {expect[k]}, got {got[k]}")
    if verbose:
        print("      painted:", repr(painted[-400:]))
    return not bad


def main():
    verbose = "-v" in sys.argv
    os.system("tmux kill-server 2>/dev/null")
    print("bidi probe - who reorders, and does the terminal agree\n")
    results = []
    for name, argv, feed, seconds, expect in CASES:
        results.append(run(name, argv, feed, seconds, expect, verbose))
        os.system("tmux kill-server 2>/dev/null")
    print("\nALL PASS" if all(results) else f"\n{results.count(False)} FAILED")
    return 0 if all(results) else 1


if __name__ == "__main__":
    sys.exit(main())

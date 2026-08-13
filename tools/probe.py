#!/usr/bin/env python3
"""Drive Claude Code inside a controlled pty and report what it draws.

Answers three questions without needing anyone to look at a screen:
  1. does Claude reorder Hebrew before writing it out (visual vs logical order)
  2. where does it put the caret on a Hebrew line
  3. which arrow escape form does it act on, and which way does the caret go
"""

import fcntl
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time

import pyte

COLS, ROWS = 100, 30
HEB = re.compile(r"[֐-׿]")
SENTENCE = "שלום עולם"


def set_size(fd, cols, rows):
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))


class Session:
    def __init__(self, argv, env):
        self.master, slave = pty.openpty()
        set_size(self.master, COLS, ROWS)
        self.screen = pyte.Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)
        self.proc = subprocess.Popen(
            argv,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            env=env,
            preexec_fn=os.setsid,
            close_fds=True,
        )
        os.close(slave)

    def pump(self, seconds):
        end = time.time() + seconds
        while time.time() < end:
            r, _, _ = select.select([self.master], [], [], 0.1)
            if not r:
                continue
            try:
                data = os.read(self.master, 65536)
            except OSError:
                break
            if not data:
                break
            self.stream.feed(data)

    def send(self, text):
        os.write(self.master, text.encode("utf-8"))

    def cursor(self):
        return self.screen.cursor.x, self.screen.cursor.y

    def line(self, y):
        return "".join(self.screen.buffer[y][x].data for x in range(COLS)).rstrip()

    def hebrew_rows(self):
        return [y for y in range(ROWS) if HEB.search(self.line(y))]

    def close(self):
        try:
            os.killpg(os.getpgid(self.proc.pid), signal.SIGTERM)
        except Exception:
            pass
        try:
            self.proc.wait(timeout=5)
        except Exception:
            try:
                os.killpg(os.getpgid(self.proc.pid), signal.SIGKILL)
            except Exception:
                pass


def show(label, s):
    print(f"\n--- {label} ---")
    rows = s.hebrew_rows()
    if not rows:
        print("no hebrew found on screen")
    for y in rows:
        text = s.line(y)
        print(f"row {y}: {text!r}")
        heb = "".join(ch for ch in text if HEB.match(ch))
        print(f"  hebrew glyphs in screen order: {heb}")
        print(f"  codepoints: {[hex(ord(c)) for c in heb]}")
    print(f"cursor: {s.cursor()}")


def main():
    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    env["TERM_PROGRAM"] = "vscode"
    env["COLORTERM"] = "truecolor"
    env["LANG"] = "en_IL.UTF-8"
    env.pop("CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT", None)
    env.pop("CLAUDE_CODE_NATIVE_CURSOR", None)

    print(f"logical order of the test sentence: {SENTENCE}")
    print(f"logical codepoints: {[hex(ord(c)) for c in SENTENCE if HEB.match(c)]}")

    s = Session(["claude"], env)
    try:
        s.pump(12)

        # Clear whatever startup dialog is in the way, then look again.
        for _ in range(3):
            joined = " ".join(s.line(y) for y in range(ROWS))
            if "safety check" in joined or "trust" in joined.lower():
                s.send("\r")
                s.pump(4)
            else:
                break

        print("\n=== screen before typing ===")
        for y in range(ROWS):
            t = s.line(y)
            if t:
                print(f"row {y}: {t!r}")

        s.send(SENTENCE)
        s.pump(3)
        show("after typing Hebrew", s)
        before = s.cursor()

        for label, seq in (
            ("CSI left  ESC[D", "\x1b[D"),
            ("SS3 left  ESC OD", "\x1bOD"),
            ("CSI right ESC[C", "\x1b[C"),
            ("SS3 right ESC OC", "\x1bOC"),
        ):
            s.send(seq)
            s.pump(1.2)
            after = s.cursor()
            print(f"{label}: cursor {before} -> {after}  dx={after[0] - before[0]}")
            before = after
    finally:
        s.close()


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Record what a multiplexer composes when two panes sit side by side.

    .venv/bin/python tools/probe7.py [tmux|herdr]

A vertical split means one buffer row carries both panes plus the divider
column. caret.js works per row, so this is the input it actually sees. The
Hebrew comes from real Claude Code composers, one per pane; nothing is
submitted.
"""

import json
import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe import COLS as _UNUSED_COLS, Session, set_size  # noqa: F401

import pyte

COLS, ROWS = 200, 40
HEB = re.compile(r"[֐-׿]")
LEFT = "שלום עולם מהפאנל השמאלי"
RIGHT = "וזה הפאנל הימני עם עברית"
CLAUDE = os.path.expanduser("~/.local/bin/claude")
SOCKET = "rtlprobe"
HERDR = os.path.expanduser("~/.local/bin/herdr")
DRIVER = sys.argv[1] if len(sys.argv) > 1 else "tmux"


class Screen(pyte.Screen):
    # tmux asks for the cursor position with the private form ESC[?6n, which
    # pyte routes into report_device_status with a keyword it does not accept.
    def report_device_status(self, *args, **kwargs):
        return None


class Wide(Session):
    def __init__(self, argv, env):
        super().__init__(argv, env)
        set_size(self.master, COLS, ROWS)
        self.screen = Screen(COLS, ROWS)
        self.stream = pyte.ByteStream(self.screen)

    def line(self, y):
        return "".join(self.screen.buffer[y][x].data for x in range(COLS))

    def rows(self):
        return [self.line(y) for y in range(ROWS)]


def main():
    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    env["TERM_PROGRAM"] = "vscode"
    env["COLORTERM"] = "truecolor"
    env["LANG"] = "en_IL.UTF-8"
    env.pop("TMUX", None)
    env.pop("CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT", None)
    env.pop("CLAUDE_CODE_NATIVE_CURSOR", None)

    if DRIVER == "tmux":
        os.system(f"tmux -L {SOCKET} kill-server >/dev/null 2>&1")
        argv = ["tmux", "-L", SOCKET, "-f", "/dev/null", "new-session", "-s",
                "p", CLAUDE]
        split = f"tmux -L {SOCKET} split-window -h -t p '{CLAUDE}'"
    else:
        argv = [HERDR, "--session", SOCKET]
        split = f"{HERDR} pane split --current --direction right"
    s = Wide(argv, env)
    try:
        s.pump(15)
        s.send("\r")          # dismiss the trust prompt if it is up
        s.pump(6)
        s.send(LEFT)
        s.pump(4)

        os.system(f"{split} >/dev/null 2>&1")
        s.pump(15)
        s.send("\r")
        s.pump(6)
        s.send(RIGHT)
        s.pump(5)

        rows = s.rows()
        out = {
            "cols": COLS,
            "rows": ROWS,
            "left_typed": LEFT,
            "right_typed": RIGHT,
            "lines": rows,
        }
        path = os.path.join(os.path.dirname(os.path.abspath(__file__)),
                            f"{DRIVER}-split.json")
        with open(path, "w", encoding="utf-8") as fh:
            json.dump(out, fh, ensure_ascii=False, indent=2)

        for y, text in enumerate(rows):
            if HEB.search(text) or "│" in text:
                print(f"row {y:2d}: {text.rstrip()!r}")
        print(f"\nwrote {path}")
        print(f"cursor: {s.cursor()}")
    finally:
        s.close()
        if DRIVER == "tmux":
            os.system(f"tmux -L {SOCKET} kill-server >/dev/null 2>&1")
        else:
            os.system(f"{HERDR} server stop >/dev/null 2>&1")


if __name__ == "__main__":
    main()

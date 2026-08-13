#!/usr/bin/env python3
"""Same pty harness, but with a long wrapping Hebrew sentence.

Reports every row of the input box with column indices so the run boundaries
Claude actually produced can be read off directly, plus the caret column.
"""

import os
import re
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe import COLS, ROWS, HEB, Session  # noqa: E402

LONG = "שלום עולם, זה משפט ארוך מאוד בעברית שאמור להישבר לכמה שורות בטרמינל, כולל פסיקים ונקודות. בדיקה 123 וגם מילה English באמצע."


def dump(s, label):
    print(f"\n--- {label} ---")
    rows = [y for y in range(ROWS) if HEB.search(s.line(y))]
    for y in rows:
        text = s.line(y)
        print(f"row {y}:")
        print(f"  raw: {text!r}")
        marks = "".join(
            "H" if HEB.match(c) else ("L" if c.isalpha() else ("." if c == " " else "n"))
            for c in text
        )
        print(f"  cls: {marks}")
        print(f"  idx: {''.join(str(i % 10) for i in range(len(text)))}")
    print(f"caret: {s.cursor()}")
    return rows


def main():
    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    env["TERM_PROGRAM"] = "vscode"
    env["COLORTERM"] = "truecolor"
    env["LANG"] = "en_IL.UTF-8"

    s = Session(["claude"], env)
    try:
        s.pump(12)
        for _ in range(3):
            joined = " ".join(s.line(y) for y in range(ROWS))
            if "safety check" in joined or "trust" in joined.lower():
                s.send("\r")
                s.pump(4)
            else:
                break

        s.send(LONG)
        s.pump(4)
        rows = dump(s, "long hebrew sentence")

        print("\n--- caret walk: 6 presses of ESC[D ---")
        for i in range(6):
            s.send("\x1b[D")
            s.pump(0.6)
            print(f"  after {i + 1}: caret {s.cursor()}")
    finally:
        s.close()


if __name__ == "__main__":
    main()

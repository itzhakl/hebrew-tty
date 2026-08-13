#!/usr/bin/env python3
"""Type Hebrew with punctuation one character at a time and record the caret.

Shows exactly which character makes the reported caret column stop advancing.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe import HEB, ROWS, Session  # noqa: E402

CASES = [
    "שלום, מה נשמע.",
    "קובץ src/auth.ts שורה 42",
    "hello שלום world",
]


def caret_row(s):
    y = s.cursor()[1]
    return s.line(y)


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

        for case in CASES:
            print(f"\n===== typing: {case!r} =====")
            for ch in case:
                s.send(ch)
                s.pump(0.35)
                x, y = s.cursor()
                print(f"  typed {ch!r:6} caret x={x:3}  row={caret_row(s)!r}")
            print(f"  FINAL row: {caret_row(s)!r}")
            print(f"  FINAL caret: {s.cursor()}")
            # clear the input for the next case
            for _ in range(len(case) + 4):
                s.send("\x7f")
                s.pump(0.05)
            s.pump(1)
    finally:
        s.close()


if __name__ == "__main__":
    main()

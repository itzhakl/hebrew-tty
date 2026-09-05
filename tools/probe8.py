#!/usr/bin/env python3
"""Type Hebrew carrying punctuation and record (typed-so-far, painted row, caret).

typing-samples.json holds no comma, period, colon or question mark, so the
caret run that ends at one was never measured. Same shape as probe4.py: the
typed prefix is the ground-truth logical text of the input line.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe import ROWS, Session  # noqa: E402

CASES = [
    "שלום, מה נשמע.",
    "היי, אני רוצה שתראה את זה",
    "האם זה עובד?",
    "בדיקה: קובץ src/auth.ts, שורה 42.",
]

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "punctuation.json")


def main():
    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    env["TERM_PROGRAM"] = "vscode"
    env["COLORTERM"] = "truecolor"
    env["LANG"] = "en_IL.UTF-8"

    samples = []
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
            typed = ""
            for ch in case:
                s.send(ch)
                s.pump(0.3)
                typed += ch
                x, y = s.cursor()
                samples.append({"typed": typed, "row": s.line(y), "caret": x})
            for _ in range(len(case) + 4):
                s.send("\x7f")
                s.pump(0.05)
            s.pump(1)
    finally:
        s.close()

    with open(OUT, "w", encoding="utf-8") as fh:
        json.dump(samples, fh, ensure_ascii=False, indent=1)
    print(f"wrote {len(samples)} samples to {OUT}")


if __name__ == "__main__":
    main()

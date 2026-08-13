#!/usr/bin/env python3
"""Type mixed Hebrew/English and record (typed-so-far, painted row, caret).

The typed prefix is the ground-truth logical text for the input line, so the
recovery step can be checked against it exactly instead of against a guess.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe import ROWS, Session  # noqa: E402

CASES = [
    "שלום hello",
    "שלום hello world ומה נשמע",
    "hello שלום world",
    "בדיקה test 42 סוף",
]

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "mixed.json")


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

#!/usr/bin/env python3
"""Type mixed Hebrew/English into Pi and record (typed-so-far, painted row, caret).

Pi paints its own visual order and walks the caret one cell left per character,
whichever direction that character runs, so its caret sits at the left end of a
Latin run rather than past it. The proxy has to place the caret itself, and this
records what it has to work from.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe import ROWS, Session  # noqa: E402

CASES = [
    "שלום hello עולם",
    "היי, זה קובץ src/auth.ts",
]

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "pi-typing.json")


def main():
    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    env["TERM_PROGRAM"] = "vscode"
    env["COLORTERM"] = "truecolor"
    env["LANG"] = "en_IL.UTF-8"

    samples = []
    s = Session(["pi"], env)
    try:
        s.pump(14)
        for _ in range(4):
            joined = " ".join(s.line(y) for y in range(ROWS))
            low = joined.lower()
            if "safety check" in joined or "trust" in low or "press enter" in low:
                s.send("\r")
                s.pump(5)
            else:
                break

        for case in CASES:
            typed = ""
            for ch in case:
                s.send(ch)
                s.pump(0.35)
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

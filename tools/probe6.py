#!/usr/bin/env python3
"""Record what Claude paints when text arrives in bulk rather than keystroke by
keystroke: a bracketed paste, and the burst-then-pause pattern that voice
dictation produces.

Both cases skip the one-character-at-a-time growth the caret recovery memo
relies on, so the painted row is captured together with the logical text that
produced it and can be checked against a fresh bidi reordering.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe import ROWS, Session  # noqa: E402

PASTES = [
    "שלום עולם",
    "שלום hello עולם",
    "בדיקה של טקסט מודבק עם מספר מילים",
    # Long enough to wrap: each visual row is reordered on its own.
    "זהו טקסט ארוך מאוד שנועד להיגלש על פני יותר משורה אחת בתיבת הקלט "
    "כדי לראות איך כל שורה מסודרת בנפרד וגם מה קורה בסוף המשפט הזה",
    "שורה ראשונה\nשורה שנייה\nשורה שלישית",
    "שלום, 42 (בדיקה) - src/auth.ts שורה 7.",
    "\n".join(f"שורה מספר {i} של טקסט ארוך מודבק" for i in range(1, 16)),
]

# Each entry is one dictation burst; the pause between them is what the
# renderer sees as a jump rather than a keystroke.
DICTATIONS = [
    ["שלום עולם", " מה נשמע"],
    ["היום אני רוצה", " לבדוק את הכיוון", " של האותיות"],
]

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "bulk.json")


def clear_input(s, n):
    for _ in range(n + 8):
        s.send("\x7f")
        s.pump(0.03)
    s.pump(0.6)


def input_row(s):
    x, y = s.cursor()
    rows = [{"y": r, "text": s.line(r)} for r in range(ROWS) if s.line(r)]
    return {"row": s.line(y), "caret": x, "y": y, "screen": rows}


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

        for text in PASTES:
            s.send("\x1b[200~" + text + "\x1b[201~")
            s.pump(2.5)
            frame = input_row(s)
            frame.update({"kind": "paste", "typed": text})
            samples.append(frame)
            clear_input(s, len(text))

        for bursts in DICTATIONS:
            typed = ""
            for burst in bursts:
                s.send(burst)
                s.pump(2.0)
                typed += burst
                frame = input_row(s)
                frame.update({"kind": "dictation", "typed": typed})
                samples.append(frame)
            clear_input(s, len(typed))
    finally:
        s.close()

    with open(OUT, "w", encoding="utf-8") as fh:
        json.dump(samples, fh, ensure_ascii=False, indent=1)
    print(f"wrote {len(samples)} samples to {OUT}")


if __name__ == "__main__":
    main()

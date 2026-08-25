#!/usr/bin/env python3
"""Record what a patched Claude Code paints for lines of known logical text.

The base direction of a line is decided from the text, and the interesting
lines are the ones where bidi rule P2 gets it wrong: a Hebrew sentence whose
first strong character is Latin, because it opens with a path, a flag or a
version number. P2 hands the whole line to the Latin side and the sentence
lays out backwards.

Claude is asked to echo each line back verbatim, and the row it paints is read
off the screen. Nothing here is written by hand - that is the whole point.

    tools/probe8.py <patched-claude> [out.json]

Needs pyte, which tools/probe.py imports; a venv is fine.
"""
import json
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe import Session, ROWS  # noqa: E402

HEB = re.compile(r"[֐-ࣿ]")

LINES = [
    "2.1.243-rtl נמחק ו-360MB חזרו",
    "npm test עובר, 654 בדיקות",
    "src/caret.js:275 כבר עושה את זה",
    "הקובץ נמחק ו-360 מגה-בייט חזרו",
    "The file was deleted and only שלום remains",
]


def main():
    exe = sys.argv[1]
    out_path = sys.argv[2] if len(sys.argv) > 2 else "test/fixtures/base-direction.json"

    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    env["COLORTERM"] = "truecolor"

    numbered = "\n".join(f"{i + 1}. {t}" for i, t in enumerate(LINES))
    prompt = (
        "Echo back EXACTLY these lines as a numbered markdown list, nothing "
        "else, no code fence, no commentary:\n" + numbered
    )

    s = Session([exe], env)
    samples = []
    try:
        s.pump(15)
        for _ in range(3):
            joined = " ".join(s.line(y) for y in range(ROWS))
            if "trust" in joined.lower() or "safety check" in joined:
                s.send("\r")
                s.pump(5)
            else:
                break
        s.send(prompt)
        s.pump(2)
        s.send("\r")
        s.pump(60)
        screen = [s.line(y) for y in range(ROWS)]
    finally:
        s.close()

    # The echoed list is painted after the prompt; a row is the answer to a
    # line when it holds that line's number and the same set of characters.
    for i, logical in enumerate(LINES):
        want = sorted(logical.replace(" ", ""))
        for row in screen:
            body = row.strip()
            if not body.startswith(f"{i + 1}.") and not body.endswith(f".{i + 1}"):
                continue
            painted = body
            for mark in (f"{i + 1}. ", f" .{i + 1}"):
                painted = painted.replace(mark, "")
            if sorted(painted.replace(" ", "")) == want:
                samples.append({"logical": logical, "painted": painted})
                break

    missing = [t for t in LINES if t not in [s["logical"] for s in samples]]
    if missing:
        sys.exit(f"not painted, cannot record: {missing}\n" + "\n".join(screen))

    json.dump({"samples": samples}, open(out_path, "w"), ensure_ascii=False, indent=1)
    open(out_path, "a").write("\n")
    print(f"{out_path}: {len(samples)} samples")


if __name__ == "__main__":
    main()

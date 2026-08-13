#!/usr/bin/env python3
"""Type Hebrew, add a space, then backspace, recording what actually changes.

Answers whether Claude deletes the space or the preceding letter, and what
caret column it reports at each step.
"""

import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from probe import ROWS, Session  # noqa: E402

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "backspace.json")

SCRIPT = [
    ("type", "שלום"),
    ("type", " "),
    ("key", "\x7f"),
    ("key", "\x7f"),
    ("type", " עולם"),
    ("key", "\x7f"),
    ("key", "\x1b[D"),
    ("key", "\x1b[D"),
    ("key", "\x7f"),
]


def main():
    env = dict(os.environ)
    env["TERM"] = "xterm-256color"
    env["TERM_PROGRAM"] = "vscode"
    env["COLORTERM"] = "truecolor"
    env["LANG"] = "en_IL.UTF-8"

    log = []
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

        for kind, payload in SCRIPT:
            s.send(payload)
            s.pump(1.0)
            x, y = s.cursor()
            label = payload if kind == "type" else repr(payload)
            entry = {"action": f"{kind} {label}", "row": s.line(y), "caret": x}
            log.append(entry)
            print(f"{entry['action']:>14}  caret={x:3}  row={entry['row']!r}")
    finally:
        s.close()

    with open(OUT, "w", encoding="utf-8") as fh:
        json.dump(log, fh, ensure_ascii=False, indent=1)
    print(f"\nwrote {OUT}")


if __name__ == "__main__":
    main()

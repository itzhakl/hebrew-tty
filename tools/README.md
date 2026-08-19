# Measurement harness

These scripts drive Claude Code inside a controlled pty and read the screen back
with a terminal emulator, so the fixtures in `test/fixtures` are recordings of
what the renderer actually produced rather than hand-written guesses.

    python3 -m venv .venv && .venv/bin/pip install pyte

| script      | what it captures                                             |
| ----------- | ------------------------------------------------------------ |
| `probe.py`  | one short Hebrew line, plus which arrow escape forms move the caret |
| `probe2.py` | a long sentence that wraps, with column indices per row       |
| `probe3.py` | punctuation and spaces typed one character at a time          |
| `probe4.py` | mixed Hebrew/English, recording the typed text alongside each frame |
| `probe7.py` | tmux split side by side, so one row carries two panes and the rule between them |

`probe4.py` writes `mixed.json`; copy it to
`test/fixtures/typing-samples.json` to refresh the ground-truth fixture.
`probe7.py` writes `tmux-split.json` the same way, for
`test/fixtures/tmux-split.json`. It needs `tmux` on PATH and starts two real
sessions, one per pane.

Each run starts a real Claude Code session and dismisses the trust prompt. No
message is ever submitted.

## `graft_hebrew.py`

Not part of the harness. It builds the monospace Hebrew font the caret fix needs
in order to land on a glyph rather than between two - see "The terminal font" in
the top-level README.

    .venv/bin/pip install fonttools
    .venv/bin/python tools/graft_hebrew.py BASE.ttf DONOR.ttf OUT.ttf FAMILY SUBFAMILY [height-ratio]

`style_alias.py` fills in a style that has no base to graft onto - typically the
italic faces - by re-labelling an upright one:

    .venv/bin/python tools/style_alias.py SRC.ttf OUT.ttf FAMILY SUBFAMILY [bold]

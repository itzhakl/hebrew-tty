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

`probe4.py` writes `mixed.json`; copy it to
`test/fixtures/typing-samples.json` to refresh the ground-truth fixture.

Each run starts a real Claude Code session and dismisses the trust prompt. No
message is ever submitted.

# hebrew-tty

Hebrew that reads right in a terminal TUI.

Claude Code lays out its own rows before they reach the terminal, so Hebrew
arrives already reordered, flushed to the wrong edge, with the caret sitting
somewhere other than the glyph it is editing. `hebrew-tty` runs the program on
a pty of its own and repairs the rows on the wire.

Nothing is patched. No binary is modified, no editor bundle is rewritten, and
an upgrade of the program inside cannot break it.

```sh
hebrew-tty claude
```

## Status

The filter is transparent today: bytes pass through untouched. What it already
proves is that the program inside sees a real terminal, gets the true window
size, survives a resize, and returns its own exit status. The row repair hooks
into one place in `bin/hebrew-tty` and nowhere else.

## Dictation

Hebrew dictation for Claude Code's `/voice`, over ElevenLabs Scribe or a local
Whisper. It needs no install and no root - the CLI builds its dictation socket
from `VOICE_STREAM_BASE_URL`.

```sh
hebrew-voice serve            # the local server
hebrew-voice -- claude        # run a command pointed at it
hebrew-voice levels           # measure the microphone and the room
```

## Layout

| path                 | role                                                    |
| -------------------- | ------------------------------------------------------- |
| `bin/hebrew-tty`     | the filter: raw stdin, pass-through, resize             |
| `tools/ptyhost.py`   | gives the child a pty; Node cannot open one unaided     |
| `src/caret.js`       | the row repair: recovery, caret mapping, mirroring      |
| `src/voice/`         | dictation: local server, Scribe or local Whisper        |
| `test/run.js`        | assertions over `test/fixtures/*.json`                  |
| `test/voice.js`      | assertions for `src/voice/`                             |
| `tools/probe*.py`    | pty probes that record the fixtures                     |

```sh
npm test
```

MIT.

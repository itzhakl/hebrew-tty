# rtl-caret

Patches xterm.js's WebGL renderer inside VS Code / VSCodium so the terminal
caret lands on the glyph it is actually editing in Hebrew and other RTL text.
Zero-dependency at runtime except `bidi-js`, which is inlined into the patch.

## Layout

| path                 | role                                                          |
| -------------------- | ------------------------------------------------------------- |
| `bin/rtl-caret.js`   | CLI: `status` / `install` / `uninstall` / `voice`, flag parsing |
| `src/patch.js`       | finds editor bundles, builds the payload, patches/reverts them |
| `src/caret.js`       | the injected logic: recovery, caret mapping, mirroring, shift  |
| `src/voice/`         | Hebrew dictation: local `voice_stream` server + Chirp providers |
| `test/run.js`        | assertion runner over `test/fixtures/*.json`                   |
| `test/voice.js`      | assertion runner for `src/voice/`                              |
| `tools/*.py`         | pty probes that record the fixtures; not shipped runtime code  |
| `tools/editor-probe.js` | drives a real patched editor over CDP; the only way to see the renderer |

## Commands

```sh
npm test                              # the whole suite
sudo node bin/rtl-caret.js install    # install from this checkout
node bin/rtl-caret.js status          # what is patched, changes nothing
node bin/rtl-caret.js voice -- claude # dictation, needs no install and no root
```

Install from the repo checkout, not a globally installed package. `sudo` loses
`node` from PATH under nvm/fnm, so pass the full node path when needed.

## Invariants

- `src/caret.js` is **embedded as text** into a third-party bundle. It must stay
  ES5, IIFE-wrapped, and free of `require`. Non-ASCII characters in it must be
  written as escapes — literals normalise on the way in.
- The caret is never moved on a guess. Recovered logical text is reordered again
  and must equal the painted line exactly; otherwise the original column stands.
- Reordering here skips bidi rule L4, because Claude Code skips it too. Use the
  manual permute, not `getReorderedString`.
- Caret mapping and row alignment must read the same per-row resolution. Two
  independent resolutions make the row flicker between alignments while typing.
- Lines with no RTL character are left untouched.
- A buffer row is not a line. A multiplexer splitting the screen side by side
  draws a rule down one column, and the row then holds two unrelated lines. The
  divider columns are found once per viewport - a rule running nearly the full
  height, which a table border never does - and span, recovery and alignment
  all run inside one pane. Alignment flushes to the pane's right edge, never
  the screen's.
- `src/patch.js` matches the bundle with regex anchors. Editor upgrades replace
  the bundle, so a failed match means "skip", never a corrupt write. Writes are
  atomic and always leave a backup.
- Two bundles are patched, and the payload in both must carry the same flags:
  the WebGL addon for the caret, mirroring and alignment, and xterm's core for
  copying. Whichever loads first wins, and the rest is skipped by the guard.
- Copying returns the logical text, so copy and paste round trip. A line that
  reorders to itself, or whose recovery does not verify, is copied verbatim -
  never guessed at.

## Voice

- The redirect is one environment variable: Claude Code's CLI builds its
  dictation socket from `VOICE_STREAM_BASE_URL`. Nothing is patched, and the
  CLI's own microphone keeps recording.
- The wire protocol belongs to Claude, not to us. Binary frames are linear16
  16 kHz mono; replies are `TranscriptInterim` / `TranscriptText` /
  `TranscriptEndpoint` / `TranscriptError`.
- **Only `TranscriptEndpoint` commits.** The client replaces a single pending
  buffer on every `TranscriptInterim`/`TranscriptText` (they are handled
  identically) and promotes that buffer on `Endpoint`. So a commit is always a
  pair, and the text sent must be the whole utterance, never a delta.
- After `CloseStream` the client arms a 1500 ms no-data timer and a 5000 ms
  safety timer. **Any frame clears the no-data timer**, which is why
  `CloseStream` is answered instantly with a keepalive interim — going quiet
  costs the accurate engine's transcript. Every settle and wait must fit inside
  5000 ms, not 1500.
- These numbers were read out of the CLI binary's `finalize()`. Re-check them
  when dictation starts truncating after a Claude Code upgrade.
- `src/voice/` must stay out of the `install` path: `require` it lazily, so the
  patch commands never load `ws` or `@google-cloud/speech`.
- Ported from the `claude-code-hebrew` extension. Fixes belonging to both should
  land there too.

## Tests

Fixtures are recordings from a real pty, never hand-written strings. Do not edit
`test/fixtures/*.json` by hand — re-record with the `tools/probe*.py` scripts.
Every new behaviour needs a fixture-backed check in `test/run.js`.

`src/voice/` has no pty recordings — the protocol is Claude's. `test/voice.js`
drives the real server over a real socket with a scripted provider instead.

## Known limitation

Under `--align`, mouse selection and link hovering address unshifted columns.
The flag stays; the fix is deferred.

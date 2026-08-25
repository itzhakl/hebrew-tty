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
| `src/voice/`         | Hebrew dictation: local `voice_stream` server + ElevenLabs Scribe |
| `test/run.js`        | assertion runner over `test/fixtures/*.json`                   |
| `test/voice.js`      | assertion runner for `src/voice/`                              |
| `tools/*.py`         | pty probes that record the fixtures; not shipped runtime code  |
| `tools/editor-probe.js` | drives a real patched editor over CDP; the only way to see the renderer |
| `tools/patch-binary.py` | patches the Claude Code executable, for terminals that are not the editor |
| `bin/claude-rtl`     | runs Claude Code from a patched build, rebuilding it after an upgrade |

## Commands

```sh
npm test                              # the whole suite
sudo node bin/rtl-caret.js install    # install from this checkout
node bin/rtl-caret.js status          # what is patched, changes nothing
node bin/rtl-caret.js voice -- Codex # dictation, needs no install and no root
```

Install from the repo checkout, not a globally installed package. `sudo` loses
`node` from PATH under nvm/fnm, so pass the full node path when needed.

## Invariants

- `src/caret.js` is **embedded as text** into a third-party bundle. It must stay
  ES5, IIFE-wrapped, and free of `require`. Non-ASCII characters in it must be
  written as escapes — literals normalise on the way in.
- The caret is never moved on a guess. Recovered logical text is reordered again
  and must equal the painted line exactly; otherwise the original column stands.
- Reordering here skips bidi rule L4, because Codex skips it too. Use the
  manual permute, not `getReorderedString`.
- Caret mapping and row alignment must read the same per-row resolution. Two
  independent resolutions make the row flicker between alignments while typing.
- Lines with no RTL character are left untouched.
- `src/patch.js` matches the bundle with regex anchors. Editor upgrades replace
  the bundle, so a failed match means "skip", never a corrupt write. Writes are
  atomic and always leave a backup.
- Two bundles are patched, and the payload in both must carry the same flags:
  the WebGL addon for the caret, mirroring and alignment, and xterm's core for
  copying. Whichever loads first wins, and the rest is skipped by the guard.
- `tools/patch-binary.py` must never match a minified name. Every site is found
  by the shape of its code and the names are read back out of the match; the
  same build renames them all. `test/binary.js` holds recordings from two
  builds that share no identifier, and a pattern that stops resolving must fail
  there rather than in a 340MB write.
- The patched executable must keep its exact length — Bun addresses the module
  graph by byte offsets — and bytes may only be moved inside one module. Since
  2.1.243 that module is a chunk with a few hundred spare bytes, so the caret
  map lives in whatever chunk has room and is reached through `globalThis`.
  Falling back to the logical column is correct; throwing is not.
- Copying returns the logical text, so copy and paste round trip. A line that
  reorders to itself, or whose recovery does not verify, is copied verbatim -
  never guessed at.

## Voice

- The redirect is one environment variable: Codex's CLI builds its
  dictation socket from `VOICE_STREAM_BASE_URL`. Nothing is patched, and the
  CLI's own microphone keeps recording.
- The wire protocol belongs to Codex, not to us. Binary frames are linear16
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
  when dictation starts truncating after a Codex upgrade.
- The engine is ElevenLabs Scribe v2 Realtime over one WebSocket:
  `input_audio_chunk` frames carry base64 PCM up, and `commit_strategy=vad`
  makes the server endpoint each utterance itself. Without VAD,
  `committed_transcript` never fires for a microphone.
- **Only `committed_transcript` ends a segment.** `partial_transcript` and
  `final_transcript` are both hypotheses - the latter is settled, not
  committed. Emitting either as a commit sends the same words twice.
- Audio is batched to ~100 ms before it goes out. One JSON+base64 frame per
  20 ms CLI frame is 50 messages a second, for latency the model cannot use.
- ElevenLabs wants a bare ISO-639-1 code, so Hebrew is `he`. A `voice.json`
  left over from the Google backend carries `iw-IL` and a Chirp model name;
  both are translated on load rather than sent as-is.
- Scribe's list parameters are **repeated** query parameters, never
  comma-joined. A comma-joined `secondary_languages` is rejected outright; a
  comma-joined `keyterms` is accepted as ONE long term, which biases the model
  at a string nobody will ever say. The live API echoes its resolved config in
  `session_started` - read it there, not in the docs, when a knob looks ignored.
- Terminal Hebrew is code-switched: paths, commands and library names arrive in
  English mid-sentence. `secondary_languages=en` is what stops them coming back
  transliterated into Hebrew letters, and it is the default. It is also the job
  the two-engine hybrid provider used to do.
- `src/voice/` must stay out of the `install` path: `require` it lazily, so the
  patch commands never load `ws`.
- Ported from the `Codex-hebrew` extension. Fixes belonging to both should
  land there too.

## Tests

Fixtures are recordings from a real pty, never hand-written strings. Do not edit
`test/fixtures/*.json` by hand — re-record with the `tools/probe*.py` scripts.
Every new behaviour needs a fixture-backed check in `test/run.js`.

`src/voice/` has no pty recordings — the protocol is Codex's. `test/voice.js`
drives the real server over a real socket with a scripted provider instead.

## Known limitation

Under `--align`, mouse selection and link hovering address unshifted columns.
The flag stays; the fix is deferred.

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
| `src/voice/`         | Hebrew dictation: local `voice_stream` server, ElevenLabs Scribe or local Whisper |
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
- The local engine is `provider: "whisper"` - faster-whisper in a Python
  sidecar (`whisper_sidecar.py`) behind a pipe, spoken to with
  `[1 byte type][4 byte big-endian length][payload]` so audio is never
  base64'd. It is started once and outlives every microphone press: loading
  the model costs about five seconds, which is not a price to pay per press.
  The venv is `~/.local/share/rtl-caret/whisper-venv`, and CUDA comes from
  pip - the sidecar dlopens `site-packages/nvidia/*/lib` itself rather than
  making the caller set `LD_LIBRARY_PATH`.
- **Whisper is PULL, Scribe is PUSH.** Whisper has no endpointer, so the local
  energy VAD decides when an utterance ended and `endSegment()` returns the
  text; `onFinal` is never called. Wiring it as a push provider would commit
  nothing until the microphone stopped.
- **Fed silence, Whisper invents a sentence** - in Hebrew, reliably
  "תודה רבה". That is not an edge case: the tail of every microphone press is
  silence. Silero (`vad_filter`) in front of the decoder is the only thing that
  tells the two apart - measured against noise at four levels, `no_speech_prob`
  came back 0.0 and `avg_logprob` looked like ordinary speech. It also pays for
  itself: a buffer with no voice in it skips the encoder, 30 ms instead of 600.
  An energy floor cannot do this job - the room clears it.
- Silero and the endpointer answer different questions. The endpointer asks
  "did the level drop", Silero asks "was that a voice". Neither substitutes for
  the other, and the endpointer must stay the only holder of absolute levels -
  two places holding the same threshold is how they drift.
- Whisper's default temperature is a fallback ladder: a decode whose logprob or
  compression ratio looks wrong is retried at 0.2, 0.4 ... 1.0, each retry a
  full pass. Half an utterance trips it constantly, which turned a 470 ms
  hypothesis into 1900 ms. Hypotheses run at a fixed `temperature=0`; only the
  commit gets the ladder.
- One card, one inference at a time. A hypothesis and a commit that overlap
  queue on the GPU and can exhaust it, so a lock serializes them and a
  hypothesis that has not started yet yields to a waiting commit.
- The encoder cost is flat: the mel is padded to thirty seconds, so twelve
  seconds of speech costs the same ~470 ms as one. Segment length is not a
  latency knob; `partialMs` and `endpointMs` are.
- **A fixed energy threshold does not survive a real microphone.** At 0.005 a
  quiet room already reads as speech, so silence never arrives, nothing ever
  commits, and dictation only lands when the key is released - which looks like
  slowness, not like a broken endpointer. `vad.js` measures the room over the
  first 300 ms of each press (Claude streams from the moment the mic opens, and
  nobody starts talking that fast) and puts the bar at three times it. The
  segment those first frames opened against the absolute threshold is taken
  back once the room is known, or it would commit itself 600 ms later.
- `noiseRatio` is not one number for every microphone. A headset hears speech
  ten times louder than its room; a laptop microphone with its gain wound up
  hears itself at 0.15 RMS with nobody in the room, and asking speech to beat
  three times that asks for more than it produces. `rtl-caret voice levels`
  records without transcribing and reports both ends, which is the only way to
  tell "the room clears the bar" from "speech does not".
- Calibration needs the real 20 ms wire frames. A caller handing over whole
  seconds at a time is not measuring a room, so it is left on the absolute
  threshold rather than told that speech is the floor.
- The hypothesis loop skips a buffer whose tail has gone quiet: the speaker has
  stopped, the text would repeat the last one, and starting it now only makes
  the commit queue behind half a second of decoding.
- `src/voice/` must stay out of the `install` path: `require` it lazily, so the
  patch commands never load `ws`.
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

# hebrew-tty

Puts Hebrew back the way it was typed in terminal coding agents. The Linux
Rust proxy owns the child PTY, VT screen model, verified execution-path
classification, per-row Unicode BiDi layout, pane alignment, repainting, and
caret map. Unknown paths pass through unchanged. Nothing inside Claude Code,
Pi, or Codex is patched. Node dependencies remain only for dictation and the
retained JavaScript regression suite.

## Layout

| path                 | role                                                          |
| -------------------- | ------------------------------------------------------------- |
| `bin/hebrew-tty`     | compatibility launcher for a built or packaged Rust binary     |
| `src/platform/linux.rs` | Linux PTY transport, signals, resize, and stream integration |
| `src/terminal.rs`    | VT screen cells, styles, cursor, panes, dirty rows, and reflow  |
| `src/classify.rs`    | fail-safe measured execution-path classification                |
| `src/layout.rs`      | logical recovery, per-row BiDi, mirroring, alignment, caret map |
| `src/render.rs`      | dirty-row repaint and mapped-caret restoration                  |
| `src/caret.js`       | predecessor engine retained as JavaScript regression evidence   |
| `bin/hebrew-voice`   | dictation entry point                                          |
| `src/voice/`         | Hebrew dictation: local `voice_stream` server, ElevenLabs Scribe or local Whisper |
| `test/run.js`        | assertion runner over `test/fixtures/*.json`                   |
| `test/voice.js`      | assertion runner for `src/voice/`                              |
| `tools/probe*.py`    | pty probes that record the fixtures; not shipped runtime code  |
| `tools/patch-binary.py` | patches the Claude Code executable; the Hebrew you see today comes from here, until the filter repairs rows itself |
| `bin/hebrew-tty-build` | makes the patched build for whatever version `claude` points at; run in the background after an upgrade |

## Commands

```sh
cargo test --all-targets        # Rust proxy suites
npm test                        # predecessor layout, voice, and patcher regressions
cargo build --release
bin/hebrew-tty claude           # direct proxy launch
bin/hebrew-tty pi
bin/hebrew-tty codex
node bin/hebrew-voice serve     # dictation, needs no install and no root
```

## Invariants

These are about the row repair itself. It reads nothing but the painted row -
that is what lets it run outside the program instead of inside it.

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
- Copying is the terminal's, not ours. The terminal holds the screen and
  copies what it painted, so a filter on the wire cannot hand back the logical
  text the way an editor patch could. Recovery still verifies rather than
  guesses; what it feeds is the caret and the repair, not the clipboard.
- The base direction is decided by counting, not by the first strong
  character. Bidi rule P2 hands a whole line to whichever side opens it, so a
  Hebrew sentence beginning with a path, a flag or a version number lays out
  left to right and its full stop lands on the wrong side. It is resolved off the recovered text: RTL when the Hebrew letters are not
  outnumbered by the Latin ones, `auto` otherwise. `auto` stays on as a second
  candidate, because a row painted by a build from before this rule has to
  stay recognisable.
- A painted row does not name one logical text. `2.1.243-rtl` and
  `rtl-2.1.243` paint the same row, so recovery can return the other one and
  copying gives it back. It verifies rather than guesses, which is the
  guarantee; being the text that was typed is not.
- Bidi rule L4 is ours to apply. Claude reorders without it, so a bracket that
  ends up inside an RTL run keeps the glyph it was typed as and points the
  wrong way. It is mirrored in one pass over the reordered line - and that array is cached per source line,
  so the pass marks itself. Mirroring twice swaps every bracket back, which
  looks exactly like never having run.
- A row carrying box drawing is not aligned. The borders of a
  table hold still because they hold no RTL, so flushing the cells to the right
  edge tears the table in half. The rule is `src/caret.js`'s `LAYOUT`. The prompt input row
  carries no box drawing - its rules are rows of their own - so it still aligns.
- A table row is not a paragraph. Every cell in it is. Reordering the row in
  one go carries the column rules along with the text, so the borders move,
  the cells land under the wrong headings, and a row whose cells are mostly
  Latin keeps an order the row above it does not. The row is cut at every vertical rule and each piece reordered against
  itself, leaving the rules where they were - the same rule that applies to a
  multiplexer's panes, one level down. Such a row carries no levels
  afterwards, so bidi rule L4 does not reach a bracket inside a table cell,
  and `src/caret.js` cannot verify its recovery either: the caret falls back
  to the logical column there and copying hands the row back verbatim.

- `tools/patch-binary.py` must never match a minified name. Every site is
  found by the shape of its code and the names are read back out of the
  match; the same build renames them all. `test/binary.js` holds recordings
  from three builds that share no identifier, and a pattern that stops
  resolving must fail there rather than in a 250MB write.
- The patched executable must keep its exact length, and since 2.1.246 that is
  no longer enough: the bytecode addresses source by offset, so a function
  whose text has moved is parsed from the wrong place and the boot dies on a
  parameter list that is not there. Each edit is therefore paid for out of the
  code that follows it, nearest first, and only the run in between shifts.
  Paying from both sides of an edit, or pooling all seven edits into one
  region, moves too much and 2.1.246 will not start.
- How far an edit may reach for its payment is not a constant to reason about.
  Too narrow and there is nothing left to shrink; too wide and the build stops
  starting. The widths are tried in order and the first whose result comes up
  is kept.
- `--version` is not a check, and neither is starting. The build that died on
  a stale bytecode offset reported its version perfectly and then refused to
  start, because printing a version touches a handful of modules and none of
  the edited ones. Worse, a build whose edits all landed in code the program
  never runs starts and paints its interface exactly like a good one - every
  call site falls back to doing nothing when its helper is missing. So the
  patcher types Hebrew at the result on a pty and reads the last cursor
  position back off the wire: near the right edge means the row was aligned,
  near the left means nothing happened. That is `works()`, and a width that
  does not pass it is not kept. `--check` runs the same question at a build
  that already exists.
- The first payment width that works is the one to keep, and it is not always
  the first tried. 2.1.246 runs out of slack at 200k and lands at 400k.
- The payment regions nest, so each one is cut from a buffer that already
  carries the edits above it. An edit reaches past the next edit to find its
  slack - at 400k a region runs 335KB - so the seven regions contain one
  another. Emitting them against the unedited buffer and applying them
  lowest-last puts back the code the higher edits replaced: only the
  lowest-offset edit survives. Every region holds its length, which is what
  lets them be applied one at a time without moving an offset.
- A build that starts is not a build that works. The one where six of seven
  edits had been reverted booted and showed its interface, because the payload
  was still injected and every call site fell back to doing nothing by design.
  Counting the `$r*_` markers catches that much - a helper that appears once
  is defined and never called - but not the case where every marker is paired
  and the code holding them is never reached. Only `works()` catches that.
- 2.1.246 does not patch. Every edit resolves and lands, and the painter they
  land in is never run: forcing its placement column to a constant, on the
  unpatched binary, leaves the screen rendering normally. The row anchor has
  to be moved onto whatever paints rows now. Until then the patcher produces
  no build for it and the shell function stands on 2.1.245-rtl.

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
  base64'd. It outlives every microphone press: loading the model costs about
  five seconds, which is not a price to pay per press.
  The venv is `~/.local/share/rtl-caret/whisper-venv` (the path predates the rename and is
  left alone: moving it costs a five second model reload for nothing), and CUDA comes from
  pip - the sidecar dlopens `site-packages/nvidia/*/lib` itself rather than
  making the caller set `LD_LIBRARY_PATH`.
- **The model is resident, not immortal.** It is ~1.3 GB, and on a desktop that
  dictates a few times a day the kernel swaps it out long before the next press
  - so "stays loaded" was already costing a disk read without saving one.
  `whisper.idleUnloadMs` (default ten minutes, `0` to opt out) ends the sidecar
  after a quiet stretch; `_ensure()` reloads it on the next press. The window is
  floored at a minute so it can never unload between two sentences of one
  thought, and the timer is `unref`'d - a pending unload must never be the
  reason a one-shot CLI run refuses to exit.
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
- Nothing is on screen until the first hypothesis lands, so what the user calls
  slowness is that one number: the audio the hypothesis waits to accumulate,
  plus one decode. The commit's own decode is not felt - the last hypothesis is
  already painted by the time it runs, and it is what carries the words spoken
  after that hypothesis started. Committing the last hypothesis instead would
  drop the end of every sentence.
- `endpointMs` is not free to shorten. At 350 ms an ordinary gap between two
  words ended the sentence: the buffer was emptied, the hypothesis started over,
  and the first text arrived 500 ms LATER than at 450. A commit of zero
  characters in the log is that, and it costs more than the shorter endpoint
  saves.
- A hypothesis is allowed on less audio than a commit - `MIN_PARTIAL_BYTES`,
  0.2 s against the commit's 0.5. Half a second of floor is half a second of
  talking to a screen that has not moved, and a bad hypothesis is replaced.
- **The second model is a knob with nothing to put in it.** `whisper.partialModel`
  runs hypotheses on a small model while the commit stays accurate - measured,
  `faster-whisper-small` hypothesises in 137 ms against the turbo's 550. It is
  off because no small Hebrew model exists: every ivrit-ai release is large, and
  a multilingual small invents Hebrew. `medium` does not fit beside the turbo on
  a 4 GB card. Quantising a second copy is not the way round it either - int8
  buys 13% (467 ms against 535), not a second channel.
- `int8_float16` is what `computeType: auto` already resolves to on a card, and
  a config that names `float16` is giving up a gigabyte for nothing. Measured on
  the same recording, the two return the same transcript word for word - the
  hard case included, a Hebrew sentence carrying `git`, `npm` and `deployment` -
  while int8 holds 1241 MiB against 2233 and decodes no slower.
- **A correction pass over the committed text does not fit on this card.** The
  idea is sound - punctuation, and the English terms that come back
  transliterated - but the room left beside whisper is ~2.7 GB, and what fits
  there cannot do the job. Measured: Gemma 3 4B took 3.3 s, left every
  transliterated term in Hebrew and deleted words that were there; Gemma 3 1B
  took 1.2 s and answered the sentence instead of correcting it ("I'm sorry, I
  can't do that"). That answer would have landed in the user's input box. The
  model that could do this is one the card cannot hold at the same time as
  whisper, and swapping them costs whisper's four second load per sentence.
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
  three times that asks for more than it produces. `hebrew-voice levels`
  records without transcribing and reports both ends, which is the only way to
  tell "the room clears the bar" from "speech does not".
- Calibration needs the real 20 ms wire frames. A caller handing over whole
  seconds at a time is not measuring a room, so it is left on the absolute
  threshold rather than told that speech is the floor.
- The hypothesis loop skips a buffer whose tail has gone quiet: the speaker has
  stopped, the text would repeat the last one, and starting it now only makes
  the commit queue behind half a second of decoding.

## Tests

Fixtures are recordings from a real pty, never hand-written strings. Do not edit
`test/fixtures/*.json` by hand — re-record with the `tools/probe*.py` scripts.
Every new behaviour needs a fixture-backed check in `test/run.js`.

`src/voice/` has no pty recordings — the protocol is Claude's. `test/voice.js`
drives the real server over a real socket with a scripted provider instead.

## Known limitations

- Mouse selection and link hovering address unshifted columns wherever a row
  is flushed to the right edge. Deferred.
- Copying gives back the painted row, not the logical text. That is the
  terminal's to decide and a filter on the wire cannot reach it.
- Claude splits a row into one write op per coloured run, so a highlighted RTL
  line arrives as two overlapping ops. Repairing them separately paints one
  over the other. Dictation hits this every press - the interim transcript is
  a dim highlight for as long as the microphone is open - and merging the ops
  needs a screen model on this side.

# hebrew-tty

Herdr reads the pane's own foreground process, and that process is the proxy,
never the agent. The agent runs on the inner PTY the proxy opened, which Herdr
cannot see at all - so a pane launched through the proxy detected as no agent
and lost every Herdr feature keyed to one. `--as` names the proxy itself with
`PR_SET_NAME`, not just the child. The name is cut to 15 characters, which is
all `comm` holds.

`--as` names the agent for the classifier too, not just for Herdr. A launcher
that resolves `claude` to `versions/2.1.252` hands over a path whose file name
is a version number, which matches no recording - so the whole filter switched
off for exactly the setup that needs it most.

Freeing the version number is not freeing the product. `sleep --version`
answers, and once the number stopped being compared its answer classified a
`sleep` as Claude Code. What is compared now is the version string with its
digits and dots removed: `(Claude Code)` must still be there, and a recording
that carries a bare number requires a bare number back.

A recorded agent version is a floor, not a lock. Pinning it to the exact
string meant every upgrade silently turned the whole filter off - `Auto` saw an
unverified path and passed every row through, which reads as "Hebrew stopped
working" and names no cause. The recorded order carries forward to later
versions, and the observed order and wrapping still override it the moment a
real row contradicts the recording.

The proxy no longer has to be the launcher. `--install` puts a block first in
the shell's rc file that execs every interactive shell into it, so the shell is
the child and the agent is whatever the inner pty brings to the foreground.
`tcgetpgrp` on the master names that group, `/proc` names its exe and argv, and
a launcher's interpreter offers its script as a second name - `node codex.js`
is `codex`. The verdict is asked on a thread and lands tagged by generation:
`claude --version` answers in 30ms, a pnpm launcher can take seconds, and
neither may stall the relay. The rows the new program painted before its
verdict landed are repaired then; a verdict for a program that has painted no
RTL yet repaints nothing, which is what `mark_generation` is for. Once it
paints RTL, every RTL row on screen is held to its path, the shell's leftovers
included, exactly as a direct launch does. The child carries `HEBREW_TTY=1`, which
is what keeps the inner shell from wrapping itself again, and a proxy that
cannot start execs the plain shell, because the rc block already exec'd the
shell away and an exit there closes the terminal.

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
| `src/platform/foreground.rs` | what the inner pty is running: group, exe, argv, and the names to try |
| `src/install.rs`     | `--install`: the rc block that execs interactive shells into the proxy |
| `src/terminal.rs`    | VT screen cells, styles, cursor, panes, dirty rows, and reflow  |
| `src/classify.rs`    | fail-safe measured execution-path classification                |
| `src/layout.rs`      | logical recovery, per-row BiDi, mirroring, alignment, caret map |
| `src/render.rs`      | dirty-row repaint and mapped-caret restoration                  |
| `src/relay.rs`       | forward, model, gate, repair - the transform half of the proxy  |
| `src/stream.rs`      | escape-sequence and synchronized-frame boundary of the stream   |
| `src/trace.rs`       | `HEBREW_TTY_TRACE` recording of both sides of the relay         |
| `src/bin/hebrew-tty-replay.rs` | offline replay of a recording; not shipped runtime code |
| `src/caret.js`       | predecessor engine retained as JavaScript regression evidence   |
| `test/run.js`        | assertion runner over `test/fixtures/*.json`                   |
| `tools/probe*.py`    | pty probes that record the fixtures; not shipped runtime code  |

## Commands

```sh
cargo test --all-targets        # Rust proxy suites
npm test                        # predecessor layout regressions
cargo build --release
bin/hebrew-tty claude           # direct proxy launch
bin/hebrew-tty                  # wrap $SHELL; agents are classified as they come up
bin/hebrew-tty --install        # do that from the rc file of every interactive shell
HEBREW_TTY_TRACE=/tmp/t.trace bin/hebrew-tty claude   # record both sides
hebrew-tty-replay /tmp/t.trace 68 132                 # replay one; REPLAY_RERUN=1 re-runs the relay
bin/hebrew-tty pi
bin/hebrew-tty codex
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
- Lines with no RTL character are left untouched, except when a prose continuation inherits a verified RTL base and alignment from its visible pane-local paragraph anchor. That exception changes placement and base direction only; it does not reverse Latin glyph order.
- Unstyled Markdown code starts a paragraph at a literal tab or exactly four leading spaces. The visible terminal snapshot cannot distinguish an expanded tab from right-alignment padding, so ambiguous wider indentation remains prose.
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
- Nothing of ours is written inside a synchronized update. Claude wraps a
  frame in `CSI ? 2026 h` ... `l`, the terminal holds the whole frame back and
  applies it at once, and the frame is painted differentially - it rewrites
  only the cells Claude believes changed. A repaint injected between two pty
  reads of one frame is applied with the frame, and the cells the frame does
  not rewrite keep what we put there. Neither side ever repaints them again:
  Claude's screen says they are already right, and ours says the same. That is
  the smear where a row reads `───── 104 +    if !text.len()... ───────`, half
  rule and half a diff line from higher up the transcript. `StreamBoundary`
  counts a frame as not-ground, so the repair waits for `l` exactly the way it
  already waits for the end of a split escape sequence. Measured on a recorded
  session: 196 of 249 pty reads painted a screen the proxy did not mean, and
  none once the frame is respected.

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

- The Claude Code executable is not patched, and there is no patcher any more.
  `tools/patch-binary.py`, `bin/hebrew-tty-build` and `test/binary.js` are gone
  with it. `claude` is whatever the launcher resolves, latest and untouched,
  and the row repair is the proxy's alone. What ended it was 2.1.246: every
  edit resolved and landed in a painter the build no longer runs, so from that
  version on no patched build was ever produced - the versions directory holds
  nothing but `-rtl.failed`. A patch that has to find seven sites by the shape
  of their minified code, pay for each edit out of the bytes that follow it so
  the file keeps its exact length, and then be typed at on a pty to prove the
  program still runs the code it edited, costs an afternoon per upgrade for a
  result the filter gives for free. The history is in `git log` if it is ever
  needed again.

## Tests

`test/fixtures/terminal-proxy/traces/*.trace` are `HEBREW_TTY_TRACE`
recordings: `<` is what the child wrote, `>` is what the terminal received,
`r` is a resize. `tests/synchronized_update.rs` re-runs one through the real
`Transform` and holds every row against what the proxy means to paint.

Fixtures are recordings from a real pty, never hand-written strings. Do not edit
`test/fixtures/*.json` by hand — re-record with the `tools/probe*.py` scripts.
Every new behaviour needs a fixture-backed check in `test/run.js`.

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

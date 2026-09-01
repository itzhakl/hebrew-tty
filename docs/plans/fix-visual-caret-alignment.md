# fix/visual-caret-alignment

**Status:** done, verified live 2026-09-01

## Goal

- G1: In `RecoverVisual`, put the caret on the glyph it belongs to after the row
  has been flushed to the pane's right edge.

## Symptom

The row is aligned correctly and the caret is not. It stays at the column the
agent asked for, while the glyphs it points at have moved right.

## What is already fixed and merged

These landed in the same sitting and are not part of this task. They are listed
because the caret was invisible behind them - each one had to be fixed before
the next became observable.

| PR | Fix |
| -- | --- |
| #35 | The child PTY inherits the parent working directory. `portable_pty` defaults an unset cwd to `$HOME`. |
| #36 | The paragraph layout reached `main`. #34 was stacked on `fix/pi-visual-caret` and merged into that base ten seconds after #33 had squashed it into `main`, so the work never arrived. |
| #37 | The recorded agent version is a floor, not an exact match, and `--as` renames the proxy so Herdr detects the pane. |
| #38 | The classifier reads the `--as` name instead of the versioned build path, and the version string must still name the same product. |

## Evidence

Recorded from a real Claude Code 2.1.252 on an isolated PTY under `setsid`, 100
columns, typing `שלום עולם` into the prompt without pressing Enter.
`docs/plans/visual-caret-evidence/live_probe.py` reproduces it; run it as
`python3 live_probe.py passthrough` and `python3 live_probe.py auto`.

**E1 - Claude paints the input row from the LEFT, in visual order.**
`live-passthrough.txt`:

```
[24Bש  →  [24Bלש  →  [24Bולש  →  [24Bםולש  →  [24Bםלוע םולש
caret: \x1b[25;11H  →  \x1b[25;12H
```

The glyph run grows leftward on screen and the caret advances rightward,
column 11 then 12.

**E2 - The proxy aligns the row and leaves the caret behind.**
`live-auto.txt` carries the padding run that pushes the text to the right edge,
and still emits `\x1b[25;12H` - the same column as passthrough.

**E3 - The alignment engine itself is correct.** `caret_probe.py`, 60 columns:
the row is repainted with 51 leading spaces, so the glyphs occupy columns 52-60,
and the caret is written as `\x1b[10G`.

## Cause

`src/render.rs`, in `mapped_cursor`:

```rust
if disposition != RowDisposition::TransformLogical {
    return original;
}
```

`RecoverVisual` returns the caret untouched, always. That was deliberate in #33
for Pi, which emits visual-order coordinates that must not be remapped.

## The caret is not the row's shift

The first attempt moved the caret by the row's alignment shift. Measured live,
it lands on the wrong end of the run: the row is flushed to columns 91-99 and
the caret arrives at 99, which is the FIRST grapheme typed, not the insertion
point. Typing then walks it rightward, away from the text - which reads as
"it behaves like English".

An RTL run grows leftward. The next grapheme is painted at the run's left
edge, so that is where the caret belongs: column 90 here, moving left by one
per character. Pi's live measurement recorded exactly that shape - column 261,
then 260.

What the agents report is the other end. Claude paints the run from column 3
and reports column 11, the run's right edge. It is counting graphemes as an
LTR advance. So the reported column cannot be shifted, mapped or round-tripped
into the right answer; it only tells us the caret is at the end of the text.

The anchor is measured from the two rows we already hold: the width of the RTL
run that ends at the reported column in the painted row, subtracted from the
last painted column of the same row after layout. Claude: 99 - 9 = 90. Pi's
unit fixture: 64 - 4 = 60. Blanks inside the run count (the space between two
Hebrew words), blanks before the prompt marker do not - which is why the
anchor is not simply the first non-blank column, where `❯` would take the
caret to 87.

## Constraints

- C1: Do not regress the two Pi tests. `visual_paragraph_continuation_
  preserves_physical_cursor` is back at its recorded column 7 untouched: its
  caret is not at the run's right edge, so the rule does not fire.
  `visual_order_cup_columns_remain_physical_while_typing_rtl` moves to 60 and
  59, and its feed now clears the row - without `\x1b[2K` the second feed left
  a stale glyph in column 62 and the row it asserted on was never one Pi
  paints. Pi's live measurement is unaffected: it moves the caret left by one
  per grapheme, which is what the rule produces.
- C2: The caret is never moved on a guess. Three measured conditions must hold
  or the column stands: the row's recovery resolved (it has a coordinate map),
  the reported column is the last painted glyph of that row, and the RTL run
  has a non-zero width that fits inside the pane.
- C3: New behaviour needs a fixture-backed check. The regression replays the
  recorded byte stream from `docs/plans/visual-caret-evidence/`.

## Steps

- [x] 1. Evidence recorded from a live Claude Code 2.1.252 on an isolated pty:
      `docs/plans/visual-caret-evidence/`. [C3]
- [x] 2. Discriminator found - see above. It is not the shift; it is the width
      of the RTL run against the laid-out row. [G1, C1, C2]
- [x] 3. `recovered_visual_cursor` in `src/render.rs`, with regressions for
      both agents. [G1, C1, C3]
- [x] 4. Verified live: same probe, Claude Code 2.1.252, 100 columns. The row
      paints to 91-99 and the proxy emits `\x1b[25;91H` - column 90, the run's
      left edge - and the caret walks 92, 91 leftward as the text grows. [G1]

## Verification notes

- Run the suite as `setsid cargo test --all-targets`. From an interactive
  terminal 4 `cli` tests fail on an unrelated cause:
  `crossterm::terminal::size()` falls back to `/dev/tty` when stdout is a pipe
  and picks up the real terminal instead of the 24x80 they assert.
- Never run a TTY-owning binary inside a live agent session. It takes raw mode
  on the shared terminal and kills the session. Everything here runs under
  `setsid` on its own PTY.
- Rebuilding `target/release/hebrew-tty` replaces the file a running session is
  executing. Build when no session depends on it, or accept restarting them.

## Separate open items

Not part of this task; recorded so they are not rediscovered.

- **Herdr shows the wrong directory for an agent.** Not a proxy bug. Every
  process cwd measured correct after #35. Herdr renders `identity_cwd`, which
  sits on the workspace object in `~/.config/herdr/session.json` and is fixed
  when the workspace is created - `/home/itzhakl` for `w1G` - while the panes
  themselves carry the right cwd. Opening the workspace from the project is the
  clean answer; Herdr is a binary here, with no source to change.
- **Text bleeding into the input area.** Reported while the filter was switched
  off entirely, and not re-checked since #38 made it run. Confirm it still
  happens before investigating.
- **`~/.bashrc` carries the pre-Rust launcher.** Lines 59-102 hold the same
  shadowing alias and the same dead `node` invocation that `~/.zshrc` had. The
  login shell is zsh, so it was left alone.

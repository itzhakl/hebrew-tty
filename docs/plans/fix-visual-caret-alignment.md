# fix/visual-caret-alignment

**Status:** fix landed 2026-09-01, live verification (step 4) outstanding

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

## Why the obvious fix looked wrong, and was not

Offsetting the caret by the row's alignment shift fails two tests that were
read as Pi's recorded behaviour:

- `caret_mapping::visual_order_cup_columns_remain_physical_while_typing_rtl`
- `caret_mapping::visual_paragraph_continuation_preserves_physical_cursor`

Measured, both rows move by exactly the shift the caret was denied. In the
first, the glyph run goes from columns 59-62 to 61-64 while the caret stayed at
62; in the second the continuation row goes from 0-6 to 13-19 while the caret
stayed at 7. So those two constants were recording this bug at those widths, not
Pi. What Pi's own measurement recorded (`fix-pi-visual-caret.md`, step 3) is the
caret sitting beside the newest grapheme, taken at a width where the row was
already flush and the shift was zero - which the shift preserves, because a row
we did not move carries a zero offset.

Claude and Pi turn out to place the caret identically: on the column of the
newest grapheme. `live-passthrough.txt` shows it - the run is painted from
column 3 and the caret walks 3, 4 ... 11 with it. The only difference is that
Pi paints its rows already flushed right and Claude paints them at the left, so
only Claude's rows get a non-zero shift from us.

## Constraints

- C1: Do not regress the two Pi tests above. RESOLVED - see above; their
  constants moved by the same amount as their rows, and Pi's live measurement
  is untouched because its offset is zero.
- C2: The caret is never moved on a guess - the existing invariant in
  `CLAUDE.md`. Honoured: the offset is non-zero only on a row whose recovery
  resolved and which we actually re-painted; a fallback row carries zero and
  leaves the column standing.
- C3: New behaviour needs a fixture-backed check, not a hand-written string.
  `test/fixtures/*.json` are recorded by `tools/probe*.py`.

## Steps

- [x] 1. Evidence recorded from a live Claude Code 2.1.252 on an isolated pty:
      `docs/plans/visual-caret-evidence/`. The new regression replays that
      recorded byte stream rather than a hand-written string. [C3]
- [x] 2. Discriminator found: the alignment the proxy performed is the `offset`
      `layout_logical_row` already computes. It is now carried out on
      `LayoutResult::align_offset`, so the caret path can ask what the layout
      did. An alignment the agent applied itself leaves nothing for us to add
      and the offset is zero. [G1, C1, C2]
- [x] 3. `mapped_cursor` shifts a `RecoverVisual` caret by that offset, clamped
      to the pane. Regressions: the two Pi tests at their measured columns, and
      `recovered_visual_rows_that_were_flushed_right_carry_their_caret` for
      Claude. [G1, C1, C3]
- [ ] 4. Verify on a live Claude in an isolated workspace, the way
      `fix-rtl-paragraph-layout.md` did at its step 5. [G1]

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

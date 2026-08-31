# fix/visual-caret-alignment

**Status:** open, evidence recorded 2026-09-01

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

## Why the obvious fix fails

Offsetting the caret by the row's alignment shift - the difference between the
first glyph column before and after layout - fixes Claude and breaks Pi. Two
tests encode the Pi behaviour and both fail under it:

- `caret_mapping::visual_order_cup_columns_remain_physical_while_typing_rtl`
- `caret_mapping::visual_paragraph_continuation_preserves_physical_cursor`

Their fixtures hold Hebrew already sitting near the right edge, columns 60-63 of
65: Pi aligns its own output, Claude does not. Flushing an already-aligned row
still moves it a column or two, and a blunt shift follows that move and
overshoots.

So the rule is neither "always keep the physical column" nor "always shift". It
has to separate *the row moved because we aligned it* from *the row was already
where it belongs*.

## Constraints

- C1: Do not regress the two Pi tests above. They are the recorded behaviour of
  a second agent, not an implementation detail.
- C2: The caret is never moved on a guess - the existing invariant in
  `CLAUDE.md`. A shift that cannot be justified from the row leaves the original
  column standing.
- C3: New behaviour needs a fixture-backed check, not a hand-written string.
  `test/fixtures/*.json` is recorded with `tools/probe*.py`.

## Steps

- [ ] 1. Turn `live-passthrough.txt` and `live-auto.txt` into recorded fixtures
      through `tools/probe*.py`, at two widths as the existing pairs do. [C3]
- [ ] 2. Find the discriminator between an alignment the proxy performed and one
      the agent had already applied. The layout result knows what it did; the
      caret path currently does not ask. [G1, C1, C2]
- [ ] 3. Apply the shift only in the first case, and add regressions covering
      both agents. [G1, C1, C3]
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

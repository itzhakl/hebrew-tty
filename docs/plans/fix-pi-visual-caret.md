# fix/pi-visual-caret

**Status:** approved 2026-08-30

## Goal
- **G1** — Keep the caret attached to Hebrew input and moving right-to-left in Pi inside Herdr/Ptyxis.

## Context
- **CX1** — Pi 0.84.4 on the verified Herdr path emits visual-order, right-aligned Hebrew with a correct physical cursor trajectory (`63 → 62`), but the proxy interprets those columns as logical coordinates and repaints them as `5 → 6`.

## Constraints
- **C1** — Preserve the child-reported physical cursor for verified visual-order output; retain coordinate remapping for logical-order output.
- **C2** — Do not patch Pi, Herdr, or Ptyxis, and keep unsupported paths in safe passthrough.
- **C3** — Avoid changing glyph order, alignment, wrapping, styles, or hyperlinks while correcting the caret.

## Done when
- **D1** — A regression test reproduces a leading-padded visual Hebrew input row and proves that appending Hebrew moves the caret one column left — proven by `cargo test --all-targets`.
- **D2** — Pi 0.84.4 launched through the Herdr plugin in Ptyxis visibly keeps the caret beside the last Hebrew grapheme and moves it right-to-left — proven by a live pane check.

## Plan

**Outcome:** Visual-order agents retain their physical cursor coordinates while logical-order agents continue using logical-to-visual caret mapping. [G1]

## Acceptance

| # | Requirement | Proof | State |
|---|---|---|---|
| 1 | Appending Hebrew on a leading-padded visual row moves the caret one column left without changing row layout. [D1, C1, C3] | `cargo test --test caret_mapping visual_order_cup_columns_remain_physical_while_typing_rtl -- --exact` | passing |
| 2 | Existing logical caret mapping, layout, wrapping, styles, hyperlinks, and safe passthrough remain intact. [C1, C2, C3, D1] | `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-targets` | passing |
| 3 | Pi 0.84.4 in Herdr/Ptyxis keeps the caret beside the newest Hebrew grapheme and moves it right-to-left. [D2] | Isolated `hebrew-tty-tests` workspace: raw and repainted CUP matched at column 261, then 260 | passing |

## Files and interfaces

- `src/render.rs` — use the selected `RowDisposition` to preserve physical cursor coordinates for `RecoverVisual` and map only `TransformLogical`; public renderer interfaces remain unchanged. [G1, C1, C2, C3]
- `tests/caret_mapping.rs` — reproduce the observed visual CUP trajectory `63 → 62` and retain existing logical mapping assertions. [D1, C1, C3]
- `docs/plans/fix-pi-visual-caret.md` — record observed acceptance evidence. [D1, D2]

## Steps

- [x] 1. Add the focused visual-order regression and confirm it fails with the observed `5 → 6` remap. [G1, D1]
- [x] 2. Make renderer cursor restoration disposition-aware, preserving physical coordinates for visual/pass-through rows and retaining mapping for logical rows; run the focused caret suite. [G1, C1, C2, C3, D1]
- [x] 3. Run formatting, linting, and all Rust targets; update automated acceptance rows from observed evidence. [C2, C3, D1]
- [x] 4. Rebuild the release binary, open a fresh Pi 0.84.4 pane through the Herdr plugin in Ptyxis, verify leftward caret movement, and update live acceptance evidence. [D2]

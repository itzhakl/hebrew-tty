# fix/rtl-paragraph-layout

**Status:** approved 2026-08-30

## Goal
- G1: Render every continuation row using the base direction and alignment of its paragraph's first row.

## Context
- CX1: Pi redraws Markdown wrapping as separate absolute-positioned terminal rows, so the proxy currently resolves each physical row independently and produces visually inconsistent Hebrew paragraphs.

## Constraints
- C1: Derive paragraph membership per pane from the complete visible viewport; the top visible row is a hard boundary and no state persists offscreen.
- C2: Blank rows, new list items, independently recognizable code rows, standalone URLs, tables/separators, prompts, and UI rows terminate paragraphs and remain independently laid out.
- C3: Every prose continuation inherits the paragraph-start direction and alignment, including RTL-free English continuation rows.
- C4: Preserve glyph order, styles, hyperlinks, wrapping, caret behavior, pane isolation, and fail-safe passthrough.

## Done when
- D1: Regression tests prove Hebrew and English continuation rows inherit an RTL paragraph's right alignment while independent code, URL, list, table, prompt/UI, and viewport-top rows do not.
- D2: A Pi Markdown response in an isolated Herdr/Ptyxis workspace visibly keeps each paragraph consistently aligned without regressing the corrected caret.

## Plan

**Outcome:** Pane-local visible prose paragraphs share their anchor's base direction and alignment across hard TUI rows, while explicit boundaries remain independent. [G1]

## Acceptance

| # | Requirement | Proof | State |
|---|---|---|---|
| 1 | Hebrew and RTL-free English hard-row continuations inherit an RTL paragraph anchor without changing glyph order, styles, hyperlinks, or wrapping. [D1, C1, C3, C4] | `cargo test --test screen_layout hard_prose_rows_inherit_the_anchor_layout -- --exact` | passing |
| 2 | Blank rows, new list items, code, standalone URLs, tables/separators, prompts/UI, pane boundaries, and viewport top prevent inheritance. [D1, C1, C2, C4] | Focused `screen_layout` boundary, viewport, pane, and passthrough tests | passing |
| 3 | Anchor-dependent repainting and the physical visual-order caret remain correct. [C4, D2] | Focused `caret_mapping` paragraph and existing visual CUP tests | passing |
| 4 | Pi in an isolated Herdr/Ptyxis workspace renders consistent paragraph alignment and retains leftward Hebrew caret movement. [D2] | Isolated workspace `hebrew-tty-paragraph-tests`, 67-column Pi pane: Hebrew rows ended at the right edge; the English-only continuation final short row had lead=46/trail=1 (right edge); fenced code, URL, and table remained left/independent; the final Hebrew paragraph returned right. User confirmed the corrected caret, and automated visual caret/repaint tests passed. Workspace was closed after capture. | passing |

## Files and interfaces

- `src/layout.rs` — derive private viewport/pane-local paragraph policies, classify conservative boundaries, and force inherited output base/alignment without changing public layout interfaces. [G1, C1, C2, C3, C4]
- `tests/screen_layout.rs` — cover hard-row inheritance, boundary reset, viewport top, pane isolation, styles/hyperlinks, wrapping, and safe passthrough. [D1, C1, C2, C3, C4]
- `tests/caret_mapping.rs` — protect physical visual-order cursor coordinates and dependent-row repainting. [D1, D2, C4]
- `CLAUDE.md` — document the approved exception for RTL-free prose continuations inside visible RTL paragraphs. [C3, C4]
- `docs/plans/fix-rtl-paragraph-layout.md` — record observed automated and live evidence. [D1, D2]

## Steps

- [x] 1. Add private paragraph policy plumbing for existing soft-wrap groups while preserving current behavior and public interfaces. [G1, C1, C4]
- [x] 2. Add conservative pane-local boundary classification and hard-row inheritance, including RTL-free prose continuations. [G1, C1, C2, C3, C4]
- [x] 3. Add focused layout, boundary, viewport, pane, passthrough, repaint, and caret regressions. [D1, C1, C2, C3, C4]
- [x] 4. Update the invariant documentation and run formatting, linting, all Rust targets, npm regressions, and release build. [D1, C3, C4]
- [x] 5. In a fresh isolated Herdr/Ptyxis workspace, verify paragraph inheritance, independent boundaries, viewport behavior, and the corrected Pi caret; update acceptance from observed evidence. [D2, C1, C2, C3, C4]

Report path and no other changes.

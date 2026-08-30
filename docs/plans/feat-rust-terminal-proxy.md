# feat/rust-terminal-proxy

**Status:** approved 2026-08-30

## Current state
- **C1** — `hebrew-tty` currently launches a child through a Python-owned PTY but forwards its output bytes unchanged; the existing JavaScript row-recovery engine is not connected to that stream (read: `bin/hebrew-tty`, `tools/ptyhost.py`, `src/caret.js`).
- **C2** — Herdr 0.8.2 plugins can launch executable actions and panes, but plugin v1 cannot intercept or replace rendering for arbitrary existing panes (read: Herdr 0.8.2 plugin documentation, `herdr plugin --help`).
- **C3** — The observed failures have two distinct causes to classify per execution path: duplicate BiDi reordering reverses an already-correct result, while paragraph-wide reordering before wrapping reverses the vertical order of wrapped rows (read: `docs/hebrew-in-the-claude-tui.md`, Pi RTL extension and plan).

## Goal
- **G1** — Deliver one fail-safe Rust terminal proxy that renders Hebrew and mixed-direction input correctly for Claude Code, Pi, and Codex, both standalone and when launched through Herdr.
- **G2** — Preserve correct input caret placement while text streams, wraps, and reflows after terminal resize.

## Scope
- **S1** — Replace the transparent PTY path with a Rust executable containing PTY transport, VT screen state, execution-path classification, per-row Unicode BiDi layout, right alignment, caret mapping, dirty-row repainting, diagnostics, and configuration.
- **S2** — Add a Herdr plugin whose actions launch Claude Code, Pi, and Codex through the Rust proxy, while keeping the engine usable directly outside Herdr.
- **S3** — Add measured PTY fixtures for direct and Herdr-hosted runs so every supported path records which layer wraps and which layer performs BiDi reordering.

## Non-goals
- **NG1** — The first release does not support macOS or Windows; platform boundaries must remain isolated so support can be added later.
- **NG2** — The first release does not remap mouse input, terminal selection, or copied text.
- **NG3** — The first release does not modify Herdr core or require a Herdr render hook; a direct render-hook integration remains a later upstream phase.
- **NG4** — Unknown or unverifiable output is not reordered by default.

## Approach
- **A1** — First measure each application and hosting path, then run it behind a Rust virtual-terminal proxy that owns the single allowed BiDi transformation and repaints verified visual rows; expose the same binary through Herdr plugin actions.
- **A2** — Keep classification fail-safe: verified transformations run automatically, while uncertain rows pass through unchanged and emit diagnostics; user configuration can force `auto`, `logical`, `visual`, or `passthrough` behavior per command.

## Decisions
- **D1** — Build an independent proxy plus a Herdr plugin now, then pursue a Herdr render hook later. Rejected: waiting for a Herdr core API, because the standalone tool must work outside Herdr and plugin v1 already supports executable actions.
- **D2** — Target Linux/Ptyxis in the first release with isolated platform interfaces. Rejected: immediate cross-platform PTY support, because it would expand the first delivery before the rendering model is proven.
- **D3** — Leave unverifiable rows untouched by default and allow explicit configuration overrides. Rejected: aggressive heuristic rewriting, because a false positive creates duplicate BiDi reordering and corrupts correct text.
- **D4** — Include display order, wrapping, alignment, input caret mapping, streaming updates, and resize handling in the first release. Rejected: selection, copy, and mouse remapping in the same release, because they require a larger bidirectional interaction surface.

## Verification
- **V1** — Recorded direct and Herdr-hosted PTY cases for Claude Code, Pi, and Codex classify logical versus visual output and pre-wrap versus post-wrap BiDi behavior without relying on screenshots alone.
- **V2** — Hebrew-only and mixed Hebrew/English/code prompts preserve logical top-to-bottom row order, readable per-row BiDi order, and right alignment at multiple terminal widths, proven by deterministic virtual-screen fixtures.
- **V3** — The input caret remains on the edited grapheme while typing, moving horizontally and vertically, receiving streamed insertion, and resizing, proven by coordinate-map tests and Linux/Ptyxis smoke tests.
- **V4** — Unclassified and deliberately ambiguous fixtures remain byte-for-byte pass-through in safe mode, while each configured override produces its documented behavior.
- **V5** — Herdr can link the local plugin and invoke actions that launch each supported agent through the same Rust binary; direct CLI invocation works without Herdr.

## Open
- **Q1** — [ASSUMED: use a versioned TOML configuration under the standard XDG config directory, with command-specific mode overrides; the exact schema is reversible during planning.]
- **Q2** — [DEFERRED: the later Herdr render-hook API and upstream contribution shape will be designed only after the standalone engine and plugin integration are measured.]

## Clarifications
### Session 2026-08-30
- Q: How should the Herdr integration work? → A: Build an independent Rust proxy and Herdr plugin now, then add or propose a direct Herdr render hook later (option C).
- Q: Which operating systems must the first release support? → A: Linux/Ptyxis first, with portable architecture boundaries (option A).
- Q: What should happen when the engine cannot determine confidently whether text already passed through BiDi? → A: Pass through safely by default, with configuration overrides (option A plus configuration).
- Q: What must the first release include beyond correct RTL display? → A: Display, wrapping, alignment, streaming, resize, and especially correct caret placement on input text (option A).

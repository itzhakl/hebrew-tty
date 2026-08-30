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
- **NG5** — Host pipe EOF is not synthesized inside the child PTY: a PTY has no write-half close, and injecting a control byte would corrupt raw-mode input or race later terminal-mode changes. The proxy stops reading and preserves the child session until the child exits, matching a real terminal whose input remains attached.

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

## Plan

**Done means:** one Linux Rust PTY proxy, invoked directly or by the linked Herdr plugin, measures and classifies Claude Code, Pi, and Codex execution paths, applies at most one verified per-row BiDi layout, preserves right alignment and caret coordinates through streaming, wrapping, and resize, and passes ambiguous output through unchanged with diagnostics and command overrides. [G1, G2, A1, A2, D2, D3, D4]

**Out of scope:**
- macOS and Windows PTY implementations are deferred behind the platform interface. [NG1, D2]
- Mouse coordinates, terminal selection, and clipboard reconstruction are not changed. [NG2, D4]
- Herdr core and its renderer are not modified; a render-hook proposal remains deferred. [NG3, Q2, D1]
- Unverified output is not transformed in `auto` mode. [NG4, D3]

**Epic:** standalone Rust terminal rendering proxy with Herdr launch integration [G1, S1, S2] · **Blocked by:** none [C1, C2, D1]

## Requirements

| # | requirement | how it is proven | state |
|---|-------------|------------------|-------|
| 1 | Direct and Herdr-hosted Claude Code, Pi, and Codex recordings identify logical/visual order and pre-/post-wrap behavior before transformation code is enabled. [S3, A1, V1] | `cargo test --test measurements` against probe-recorded fixtures, plus `python3 tools/terminal_proxy_probe.py verify test/fixtures/terminal-proxy/measurements` | passing |
| 2 | At supported widths, Hebrew and mixed Hebrew/English/code retain logical top-to-bottom wrapped-row order, readable per-row display order, and pane-right alignment. [G1, D4, V2] | `cargo test --test screen_layout` | failing |
| 3 | The caret coordinate stays attached to the edited grapheme through typing, horizontal/vertical movement, streamed replacement, wrapping, and resize. [G2, D4, V3] | `cargo test --test caret_mapping` and `tools/smoke-ptyxis.sh` | failing |
| 4 | `auto` leaves unclassified and ambiguous output byte-for-byte unchanged with a diagnostic, while `logical`, `visual`, and `passthrough` overrides produce their specified paths. [A2, D3, V4] | `cargo test --test safe_modes` | failing |
| 5 | Direct CLI launch and the three linked Herdr actions start Claude Code, Pi, and Codex through the same Rust executable. [G1, S2, D1, V5] | `cargo test --test cli` and `tools/verify-herdr-plugin.sh` | failing |

## Files and interfaces

- `Cargo.toml` and `Cargo.lock` — define the `hebrew-tty` Rust binary and pin PTY, VT parsing, Unicode BiDi, grapheme, width, TOML, serialization, error, and CLI dependencies for the Linux-first proxy. [S1, D2]
- `src/main.rs` and `src/cli.rs` — replace the shipped Node entry path with `hebrew-tty [--mode auto|logical|visual|passthrough] [--diagnostics PATH] [--as NAME] <command> [args...]`, load configuration, select the command policy, and run the proxy. [S1, A2, D2]
- `src/config.rs` — expose `Config::load`, `CommandPolicy`, and `Mode`; read versioned XDG TOML defaults plus command-specific overrides, with CLI values taking precedence. [A2, Q1]
- `src/platform/mod.rs` and `src/platform/linux.rs` — define `PtyHost::{spawn, resize, read, write, wait}` plus `WindowSize`; the Linux implementation owns the child session, controlling PTY, signal forwarding, raw-mode restoration, and optional argv0 used by Herdr process recognition. [S1, D2]
- `src/terminal.rs` — expose `TerminalModel::{feed, resize, take_dirty_rows, cursor}` over VT screen state; retain cell text, style, width/continuation state, cursor state, pane separators, and physical row boundaries without treating a buffer row as a paragraph. [S1, G2, D4]
- `src/classify.rs` — expose `Classifier::observe` and `ExecutionPath { order, wrapping, confidence, evidence }`; match only probe-verified command/host behavior and return `Unknown` for incomplete or contradictory evidence. [C3, A1, A2, D3]
- `src/layout.rs` — expose `layout_row(RowSnapshot, ExecutionPath, Mode) -> LayoutResult`; recover verified logical content where needed, resolve one per-row BiDi permutation, wrap before per-row display ordering, right-align inside pane bounds, and return the logical↔visual grapheme coordinate map consumed by both painting and caret placement. [C3, S1, D4, V2, V3]
- `src/render.rs` — expose `Renderer::repaint`; emit terminal mode setup and minimal dirty-row cursor/style writes, restore the mapped caret after each batch, and repaint all affected rows after resize/reflow. [S1, G2, D4]
- `src/diagnostics.rs` — emit structured records containing command, host, classification evidence, selected mode, row disposition, and safe-mode reason without changing the rendered stream. [A2, D3, V1, V4]
- `bin/hebrew-tty` — become a compatibility launcher for the built/installed Rust binary so the existing command name remains the direct entry point. [C1, G1, S1]
- `tools/ptyhost.py` — retire from the runtime path after parity tests establish Rust PTY transport, while leaving history-driven probe tooling independent of the shipped binary. [C1, S1, D2]
- `tools/terminal_proxy_probe.py` — record raw child PTY output, input events, sizes, resize events, cursor reports, host path, and expected classification for each supported agent without screenshots. [S3, A1, V1]
- `test/fixtures/terminal-proxy/measurements/*.json` — store probe-recorded direct and Herdr-hosted Claude Code, Pi, and Codex measurements at fixed widths; these fixtures are created before layout implementation and are never hand-authored. [S3, A1, V1]
- `test/fixtures/terminal-proxy/screens/*.json` — store deterministic VT event streams and expected rows/caret positions for Hebrew-only, mixed-direction/code, streaming, wrapping, resize, ambiguous, and override cases derived from the measured paths. [V2, V3, V4]
- `tests/measurements.rs`, `tests/screen_layout.rs`, `tests/caret_mapping.rs`, `tests/safe_modes.rs`, and `tests/cli.rs` — provide focused integration boundaries for recorded classification, row layout, shared coordinate mapping, fail-safe modes, PTY lifecycle, exit propagation, and the public CLI. [V1, V2, V3, V4, V5]
- `herdr-plugin.toml` — declare Linux-only Herdr 0.8.2-compatible `claude`, `pi`, and `codex` actions plus matching terminal pane entrypoints. [S2, D1, D2, V5]
- `plugins/herdr-terminal-proxy/launch.sh` — map each action id to `herdr plugin pane open` for its declared pane entrypoint, preserving the active workspace/cwd context and routing every pane command through the repository's `hebrew-tty` binary. [C2, S2, D1, V5]
- `tools/verify-herdr-plugin.sh` — link the local manifest, assert no plugin warnings, list and invoke all three actions, verify their pane commands contain the shared proxy, then unlink without modifying Herdr core. [C2, NG3, V5]
- `tools/smoke-ptyxis.sh` — run the interactive Linux/Ptyxis width/resize/caret matrix and collect cursor-position reports and diagnostics for explicit human confirmation only where deterministic PTY assertions cannot cover the terminal renderer. [D2, V3]
- `package.json`, `README.md`, and `CLAUDE.md` — package the Rust launcher/artifacts, replace obsolete Python-PTY commands and architecture notes, and document configuration, modes, diagnostics, plugin linking, Linux scope, verification, and rollback. [G1, S1, S2, A2, D2, V5]

## Decision rules

- When a row or execution path lacks complete recorded evidence, what happens? → `auto` emits a diagnostic and preserves the original bytes/cells; only an explicit command override may select another mode. [A2, D3, NG4]
- Which layer may reorder text? → The proxy performs the single transformation only for a verified path, and terminal BiDi mode is selected so no second layer repeats it. [C3, A1]
- What is the unit of wrapping and BiDi layout? → Logical content is wrapped first, then each resulting visual row is resolved and painted independently so wrapped rows keep top-to-bottom order. [C3, D4, V2]
- Which coordinate resolution controls alignment and caret placement? → One `LayoutResult` supplies both the row shift and logical↔visual grapheme map for the repaint batch. [G2, D4, V3]
- What configuration contract is implemented first? → A versioned TOML file under the XDG configuration directory contains command-specific mode overrides, with reversible schema details isolated in `src/config.rs`. [A2, Q1]
- How does Herdr integrate? → Manifest actions open manifest-declared terminal panes whose commands invoke the standalone proxy; no render hook or Herdr-core patch is introduced. [C2, D1, NG3]

## Steps

- [x] 1. `tools/terminal_proxy_probe.py`, `test/fixtures/terminal-proxy/measurements/*.json`, and `tests/measurements.rs` — implement the recording schema and capture the six direct/Herdr-hosted agent paths at multiple widths; prove the untouched recordings classify order and wrapping with `cargo test --test measurements && python3 tools/terminal_proxy_probe.py verify test/fixtures/terminal-proxy/measurements` before adding any transformation logic. [S3, A1, V1]
- [x] 2. `Cargo.toml`, `src/main.rs`, `src/cli.rs`, `src/platform/{mod.rs,linux.rs}`, `bin/hebrew-tty`, and `tests/cli.rs` — establish the smallest reviewable Rust slice: transparent Linux PTY transport, resize/signal/exit propagation, argv0 compatibility, and byte-for-byte pass-through; prove it with `cargo test --test cli passthrough`. [C1, S1, D2, V4, V5]
- [ ] 3. `src/config.rs`, `src/classify.rs`, `src/diagnostics.rs`, and `tests/safe_modes.rs` — load XDG command policies, classify only the recorded execution paths, surface evidence, and enforce `auto`/override behavior while transformation remains a no-op; prove safe fallback first with `cargo test --test safe_modes`. [A2, D3, Q1, V1, V4]
- [ ] 4. `src/terminal.rs`, `test/fixtures/terminal-proxy/screens/*.json`, and `tests/screen_layout.rs` — build VT cell/style/cursor state, pane spans, dirty-row tracking, and resize/reflow replay against measured streams without BiDi mutation; prove parsing stability with `cargo test --test screen_layout`. [S1, D4, V2]
- [ ] 5. `src/layout.rs` and `tests/screen_layout.rs` — add verified logical recovery, wrap-before-row-resolution, mixed-direction display ordering, mirroring, and pane-right alignment using one transformation per classified path; prove row order and widths with `cargo test --test screen_layout`. [C3, A1, D3, D4, V2]
- [ ] 6. `src/layout.rs`, `src/render.rs`, and `tests/caret_mapping.rs` — produce the shared grapheme coordinate map, repaint only dirty rows, restore mapped caret position, and recompute after streamed edits and resize; prove it with `cargo test --test caret_mapping`. [G2, S1, D4, V3]
- [ ] 7. `herdr-plugin.toml`, `plugins/herdr-terminal-proxy/launch.sh`, and `tools/verify-herdr-plugin.sh` — declare and exercise the Claude Code, Pi, and Codex action→pane launch paths through the same binary; prove linking and invocation with `tools/verify-herdr-plugin.sh`. [C2, S2, D1, D2, V5]
- [ ] 8. `package.json`, `README.md`, and `CLAUDE.md` — switch packaging and operator guidance to the Rust proxy, document XDG modes/diagnostics, direct and Herdr commands, supported boundaries, and rollback, while retaining the old JavaScript fixture suite as regression evidence during migration; prove both generations with `cargo test --all-targets && npm test`. [C1, G1, A2, D2, V2, V3, V4, V5]
- [ ] 9. End-to-end verification: run `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --all-targets && npm test && python3 tools/terminal_proxy_probe.py verify test/fixtures/terminal-proxy/measurements && tools/verify-herdr-plugin.sh && tools/smoke-ptyxis.sh`; update each requirement row to `passing` only with its named evidence. [V1, V2, V3, V4, V5]

# Architecture — Hebrew dictation for the terminal

## Problem & goals

rtl-caret already fixes two of the three things that make Hebrew unusable in the
integrated terminal: the caret lands on the glyph it edits, and `--align` flushes
RTL rows to the right edge. The third is input — dictating Hebrew into Claude
Code's terminal session. Anthropic's speech backend transcribes Hebrew badly
enough to be unusable, so the goal is a complete RTL terminal: right alignment,
correct caret, and a microphone that produces correct Hebrew.

The feature must not put the existing rendering patch at risk. It ships as its
own opt-in surface, off unless explicitly run.

## Approaches considered

**A. Redirect the CLI's own dictation socket (recommended).** Claude Code's CLI
builds its dictation WebSocket URL from the `VOICE_STREAM_BASE_URL` environment
variable. Point it at a local server that speaks the same protocol, and the CLI's
own microphone, UI and `/voice` command keep working — only the transcription
engine changes. No audio capture, no patching, no injected code.

**B. Capture audio inside the patched renderer.** Extend the `src/patch.js`
payload with `getUserMedia`, a hotkey listener and `term.paste()`. Rejected:
turns `src/caret.js` from pure ES5 logic into a stateful, networked,
permission-requesting module inside a third-party bundle, with two unresolved
blockers (workbench CSP against `ws://127.0.0.1`, Electron's media permission
handler) — to rebuild capture the CLI already has.

**C. Standalone recorder daemon.** Record with `arecord`/`ffmpeg`, transcribe,
type into the focused terminal. Rejected: duplicates the CLI's microphone
handling, adds system audio dependencies, and has no good way to show recording
state.

A is the approach the `claude-code-hebrew` extension already proved in
production, through `src/terminalEnv.ts` — the extension does nothing but export
that one variable into VS Code's terminals.

## Recommended approach

A local WebSocket server on `127.0.0.1`, started by a new `rtl-caret voice`
subcommand, implementing Claude Code's `voice_stream` protocol
(`/api/ws/speech_to_text/voice_stream`; binary linear16 16 kHz mono frames in,
`TranscriptText` / `TranscriptEndpoint` / `TranscriptError` JSON frames out,
`CloseStream` to finish). Audio is streamed to Google Cloud Speech-to-Text V2
(Chirp) with `iw-IL`, and the transcript is returned on the same socket. The CLI
believes it is talking to Anthropic.

Ported from `claude-code-hebrew/extension/src`, converted from TypeScript to
plain CommonJS:

| source                   | lines | role                                     |
| ------------------------ | ----- | ---------------------------------------- |
| `stt/wsServer.ts`        | 282   | protocol, commit chain, port adoption    |
| `stt/providers/chirp.ts` | 271   | Chirp 3 streaming session                |
| `stt/providers/hybrid.ts`| 98    | fast interims + accurate final           |
| `stt/vad.ts`             | 82    | energy VAD endpointing                   |

Dropped in the port: `translate` / `translatePrompt` HTTP routes (they serve the
extension's translation features), and everything touching the `vscode` module.

Nothing in `src/patch.js` or `src/caret.js` changes.

## Key decisions

**Environment delivery.** The extension used VS Code's
`environmentVariableCollection`. The CLI equivalent is a wrapper:
`rtl-caret voice -- claude` starts (or adopts) the server, exports
`VOICE_STREAM_BASE_URL=ws://127.0.0.1:<port>`, and execs the command. Also
`rtl-caret voice env` for anyone who prefers `eval` in a shell rc, and
`rtl-caret voice status` for a `/healthz` probe. The wrapper is the documented
path: it edits no dotfiles and works in any terminal emulator, not only VS Code's.

**Stack & libraries.** Plain CommonJS, no TypeScript and no build step, matching
the rest of the repo. Two runtime dependencies, `ws` and `@google-cloud/speech`,
both `require`d lazily inside the voice code path so `install`, `status` and
`uninstall` stay dependency-free. Alternatives considered: hand-rolling the
WebSocket frame codec (~150 lines to avoid `ws`, not worth it) and the V2 batch
REST API (no gRPC dependency, but loses streaming interims — the thing that makes
dictation feel live).

**Secrets.** `~/.config/rtl-caret/voice.json`, mode 0600, holding the Google
credential (API key or service-account JSON), project id, location, model and
language. Overridable by `GOOGLE_STT_CREDENTIAL` / `GOOGLE_APPLICATION_CREDENTIALS`,
the shape `tools/`-adjacent `voice-shim` already used. Replaces VS Code
SecretStorage, which has no CLI equivalent.

**Boundaries.** The server binds `127.0.0.1` only and is unauthenticated — the
CLI cannot be told to send a token, so loopback is the whole boundary. Any local
process can open the socket and consume Google quota. This is the posture the
extension ships today; recorded here rather than solved.

**Feature isolation.** `voice` is a sibling subcommand. `install` / `uninstall`
neither enable nor mention it, and the injected payload is untouched. The toggle
is running the command, plus an `enabled` key in the config file for the
`eval`-in-rc users who need an off switch without editing their rc.

**Port strategy.** Default 8765 with the existing `startWithPortFallback` +
`/healthz` adoption logic: a second `rtl-caret voice` finds the first one's server
and reuses it instead of binding a new port.

## Missing pieces

- The CommonJS port itself — the four modules above, minus their `vscode` seams.
- A config loader and a first-run path for the credential (`rtl-caret voice
  setup`, or clear instructions on the error the server returns).
- Process lifecycle: foreground for the wrapper, and a decision on whether a
  detached long-lived daemon is worth it (deferred — see open questions).
- Fixture-backed tests in the repo's plain-assert style. `vad.ts` and the
  protocol framing are pure functions; `chirp.ts` already takes an injectable
  `ChirpStreamFactory`, so its tests port directly.

## Improvements over the extension

Three worth doing in the first pass:

1. **`rtl-caret voice test`** — record a few seconds, print the transcript and the
   engine round-trip. Replaces the ad-hoc `diag-chirp.mjs` scripts and turns
   "the mic does nothing" into a one-line diagnosis.
2. **Endpointer tuning** — `maxSegmentMs` is 4000 ms, which chops long Hebrew
   sentences mid-thought. Raise it, and expose the VAD knobs in the config file.
3. **`--lang`** — the port hardcodes nothing; Hebrew is a default, not an
   assumption.

## Spikes & experiments

**Does the CLI accept a plain `ws://` base URL outside VS Code? — RESOLVED, 2026-08-13.**
Ran `VOICE_STREAM_BASE_URL=ws://127.0.0.1:8765 claude` in a system terminal (not
VS Code's) against the extension's server, and dictated Hebrew through `/voice`:
the transcript came back in Hebrew. The same run without the variable transcribes
to English, which confirms the variable — not the environment — is what redirects
the socket. The wrapper approach is validated end to end.

That was the only unknown; everything else is code already running in production
inside the extension.

## Open questions

- **Daemon or foreground?** The wrapper keeps the server alive only as long as the
  wrapped command. A detached daemon survives across sessions but needs pid
  handling and a `stop` command. Deferred until the wrapper is in use — port
  adoption already makes repeated starts cheap.
- **Does this belong in rtl-caret at all, or in its own package?** Kept here for
  now because "one complete RTL terminal" is the goal and a single repo is a
  single install. Revisit if the Google dependency starts to weigh on users who
  only want the caret fix.
- **Overlap with the extension.** Anyone running both will have two servers and
  two config sources. Settled by port adoption at runtime, not by design here.

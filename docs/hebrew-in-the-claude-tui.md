# Hebrew in the terminal

Why Hebrew reads reversed in the Claude Code TUI on a VTE terminal, what
actually causes it, and the configuration that gets both correct order in the
TUI and per-line right alignment in the shell.

Diagnosed 2026-08-18 on Fedora 44, Ptyxis 50.1 / VTE 0.84.1, Claude Code 2.1.234.

This is about the *terminal*. Inside VSCodium the xterm.js patch in this repo
handles RTL and none of the below applies.

## The one rule

**Exactly one layer may reorder the text.** Reordering twice puts it back where
it started, which is what "reversed Hebrew" is.

Who reorders is not a matter of opinion — `tools/bidi-probe.py` reads it off the
pty:

| Where Claude Code runs | What it paints | So VTE must |
| --- | --- | --- |
| straight to the terminal | **visual** order (already reordered) | **not** reorder — BDSM explicit, `ESC[8l` |
| inside tmux | **logical** order | reorder as usual — bidi stays on |
| (a plain shell, `printf`) | logical order | reorder as usual |

Claude Code inverts its own behaviour inside tmux. That is why one setting
cannot be right everywhere, and why the fix has to be conditional.

BDSM is ECMA-48 mode 8, Bi-Directional Support Mode: `ESC[8h` implicit (VTE
reorders, the default), `ESC[8l` explicit (VTE leaves the bytes alone). Query it
with `printf '\033[8$p'` and read the `CSI 8 ; Ps $ y` reply — `Ps=1` on,
`Ps=2` off.

## What is installed

### 1. `~/.claude/settings.json` — a SessionStart hook

```json
{ "type": "command", "command": "sh \"$HOME/.local/bin/claude-bidi-off\"" }
```

Hanging it off Claude Code rather than off the shell is what makes it cover
every launch path — herdr, `claude-tmux`, a plain shell, anything added later.
`tui: "fullscreen"` and `teammateMode: "tmux"` both stay on; neither had to be
given up.

### 2. `~/.local/bin/claude-bidi-off`

```sh
#!/bin/sh
[ -n "$TMUX" ] && exit 0
t=$(ps -o tty= -p "$PPID" 2>/dev/null | tr -d ' ')
[ -z "$t" ] || [ "$t" = "?" ] && exit 0
{ printf '\033[8l' > "/dev/$t"; } 2>/dev/null
exit 0
```

Three constraints, all found the hard way:

- **Inside tmux, do nothing.** Claude emits logical order there, so VTE's bidi
  has to stay on. Turning it off is what made the herdr sessions reverse.
- **Hooks run with no controlling terminal.** Opening `/dev/tty` inside a hook
  fails with *No such device or address*. The tty is resolved from `$PPID`, the
  Claude Code process that spawned the hook.
- The write has to be silent and always exit 0, or a hook error surfaces in the
  session.

### 3. `~/.bashrc`

```bash
claude() { [ -z "${TMUX:-}" ] && printf '\033[8l'; command claude "$@"; local rc=$?; [ -z "${TMUX:-}" ] && printf '\033[8h'; return $rc; }

__rtl_bidi() { case "$TERM" in xterm*|vte*|foot*) printf '\033[8h\033[?2501h' ;; esac; }
__rtl_bidi
PROMPT_COMMAND+=(__rtl_bidi)
```

The wrapper duplicates the hook's `ESC[8l` for interactive shells, and adds the
`ESC[8h` on exit that hands bidi back to the prompt.

`__rtl_bidi` is the alignment. `ESC[?2501h` is VTE's per-line paragraph
direction autodetection: each line picks its side from its own first strong
character, so Hebrew lines right-align and Latin lines stay left. Two details:

- It needs `ESC[8h` alongside it. With autodetect on and bidi off you get
  alignment without reordering, which looks exactly like the bug.
- It is sent both at startup **and** from `PROMPT_COMMAND`. Startup alone covers
  a shell that runs a script immediately; `PROMPT_COMMAND` alone misses that
  case, and re-sending before each command keeps the mode from drifting after
  anything else has touched the terminal.

`TERM` is matched because inside tmux (`screen-256color`) the escape would go to
tmux, which swallows it. A shell inside tmux therefore gets correct order from
VTE's default but no alignment.

## Not installed, on purpose

- **A `client-attached` hook in `~/.tmux.conf`.** It was tried and removed
  twice. It turns bidi off for every tmux attach, including sessions with no
  Claude Code in them, which reverses Hebrew in an ordinary shell inside tmux —
  and it is now actively wrong, since inside tmux bidi must stay on.
- **`env: { "TERM_PROGRAM": "vscode" }`** in `~/.claude/settings.json`. Removed
  during the investigation and deliberately not restored: it made Claude Code
  identify every terminal as VS Code's, and it invalidates `echo $TERM_PROGRAM`
  as a way to tell which terminal you are in. Backup at
  `~/.rtl-caret-disabled-20260818/claude-settings.json.backup`.

## Alignment inside the TUI is not available

VTE can only align what it reorders, and outside tmux it must not reorder — so
there is no combination that gives correct order *and* right alignment in the
Claude Code TUI. The shell gets both; the TUI gets correct order only.

In VSCodium the equivalent works because `src/caret.js` runs *inside* the
renderer and makes both decisions in one place. Ptyxis offers no such injection
point: the renderer is VTE, in C, and Claude Code is a compiled binary.

## Verifying

```sh
python3 tools/bidi-probe.py       # the whole matrix, PASS/FAIL, exit code
python3 tools/bidi-probe.py -v    # also dump what each case painted
```

Seven cases: Claude Code with and without tmux, in both `tui` modes, through the
`.bashrc` wrapper, and the shell itself inside and outside tmux. Each runs on a
real pty; Hebrew is typed in; the bytes painted back are inspected along with
whether `ESC[8l` reached that pty. No screen-reading, no guessing.

For the questions bytes cannot answer - is the line right-aligned, is this glyph
the rightmost one - `tools/screenshot.py` captures the screen through the
desktop portal (GNOME refuses the older `org.gnome.Shell.Screenshot` call).
Crop to a **single glyph** before deciding: reading a whole Hebrew word back out
of an image reorders it again, which is exactly the trap this whole document is
about. Temporarily raising the Ptyxis font size makes the crop legible:

```sh
gsettings set org.gnome.Ptyxis font-name 'Cascadia Mono NF 32'
```

## Ruled out — do not re-walk

- **tmux by itself.** A plain `printf` renders correctly outside tmux and inside
  a fresh session. tmux matters only because Claude Code changes behaviour in it.
- **A Claude Code upgrade.** 2.1.233 run straight out of
  `~/.local/share/claude/versions/` behaves identically to 2.1.234.
- **Konsole.** Installed but never running; every observation was Ptyxis.
- **The alternate screen.** A four-way matrix inside `ESC[?1049h` showed plain,
  `ESC[8h` and `ESC[?2501h` all correct and only `ESC[8l` reversed.
- **Fonts and fontconfig.** The grafted `CascadiaHebrew-*.ttf` faces,
  `~/.local/share/fonts/windows/`, `50-hebrew.conf` and the Ptyxis font were
  disabled all at once with no change. All restored.
- **`.bashrc` and system rc files.** Nothing in them emits a BDSM escape.
- **`tui: "default"` vs `"fullscreen"`.** Both paint visual order outside tmux.
  The mode is not what decides it. (`"inline"` is not a valid value; the two
  valid ones are `"default"` and `"fullscreen"`.)

## Debugging playbook

1. **Measure the wire, not the screen.** `tools/bidi-probe.py` answers "who
   reordered" in twenty seconds. Nearly every wrong turn here came from
   reasoning about a rendering someone described in words.
2. **`restore-session` lies to you.** Ptyxis had it on, so a "new window" could
   come back with old tabs carrying stale VTE modes. The same configuration
   tested correct and reversed within minutes because of it. Turn it off before
   testing: `gsettings set org.gnome.Ptyxis restore-session false`.
3. **Kill everything between tests**, and check:
   `pkill -x ptyxis; pkill -f ptyxis-agent; tmux kill-server`, then
   `pgrep -a -x ptyxis; tmux ls`. The X button closes a window but the tmux
   server outlives it, and a BDSM mode lives as long as the terminal.
4. **Never test with Hebrew copied through a terminal.** Copying can hand back
   visual order; echoing those bytes looks exactly like a broken renderer.
5. **Confirm a new process actually started.** Reopening a herdr pane attaches
   to the existing Claude Code process and proves nothing.
6. **Isolate settings keys with `claude --settings <file>`** rather than editing
   the global file.
7. **Ask an unambiguous question** if a human must look: "in `אבג`, which letter
   is rightmost?" beats "is it correct?" — the second was misread more than once.

## Rollback

```sh
python3 -c "
import json
p='$HOME/.claude/settings.json'
d=json.load(open(p))
h=d['hooks']['SessionStart'][0]['hooks']
d['hooks']['SessionStart'][0]['hooks']=[x for x in h if 'claude-bidi-off' not in x.get('command','')]
json.dump(d,open(p,'w'),indent=2,ensure_ascii=False); open(p,'a').write('\n')"

rm -f ~/.local/bin/claude-bidi-off
# then drop the claude() wrapper and the ESC[?2501h line from ~/.bashrc
```

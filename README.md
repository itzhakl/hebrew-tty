# rtl-caret

Puts the terminal caret on the glyph it is actually editing when you type Hebrew
(or any right-to-left script) in the VS Code / VSCodium integrated terminal.

## The problem

Claude Code detects that its host terminal does no bidi of its own — which is the
case for VS Code, because xterm.js has no bidi support — and reorders each line
itself with the Unicode Bidi Algorithm before painting it. The text comes out
readable. The caret does not: it is reported at a **logical** offset and drawn
over **visually reordered** cells, so on a Hebrew line it lands a whole run-length
away from the character you are editing.

Measured in a real pty, typing `שלום עולם` into the prompt:

```
logical codepoints typed : 5e9 5dc 5d5 5dd 5e2 5d5 5dc 5dd   (ש ל ו ם ע ו ל ם)
painted cell order       : 5dd 5dc 5d5 5e2 5dd 5d5 5dc 5e9   (ם ל ו ע ם ו ל ש)

hebrew occupies columns 3..11
caret after typing all 9 characters: column 11
```

Column 11 holds `ש`, the *first* logical character. The caret belongs at the
visual end of the text, near column 3.

This is a defect in the editor-side renderer, not in your terminal settings.
There is no configuration that fixes it: `CLAUDE_CODE_NATIVE_CURSOR` and
`CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT` change nothing, and cursor movement is not
exposed as a rebindable action.

## What this does

Patches one expression in xterm.js's WebGL renderer — the single place where the
caret column is read — and maps that column from logical to visual order.

The mapping is not a heuristic. It runs **bidi-js**, the same library Claude Code
uses, so the permutation is identical by construction rather than approximated.
Because only the painted line is available, the logical text is recovered from
it and then **verified**: the recovered text is reordered again and must equal
the painted line exactly. If it does not, the caret is left where it was.

Two things make that recovery correct in practice:

- **More than one logical text can paint identically.** `שלום hello` and
  `hello שלום` produce the same cells, one with an RTL base direction and one
  with an LTR one, and their caret maps differ. A per-row memo of the last
  resolved text picks the candidate that continues what you were already typing.
- **A directional run swallows the neutrals inside it.** `test 42` moves as a
  single block, so splitting it on the space yields a string that repaints
  correctly but maps the caret wrong.

Lines with no RTL character are never touched.

## Mirrored brackets

Claude reorders each line but stops there. Bidi rule L4 asks for one more
step: a mirrorable character that resolved to an odd level — `(`, `[`, `{`,
`<` and their partners — is drawn as its mirror. Without it, brackets in a
Hebrew run come out the wrong way round:

```
    uppercase stands for Hebrew, so it reads right to left

    logical   ABC (DEF)
    painted   )FED( CBA      the brackets kept the shape they had
    correct   (FED) CBA      L4 draws each of them as its mirror
```

Written out in Hebrew this is `(צ'אנק, הפסקה)` coming back with its
parentheses swapped end for end. The same happens to the square brackets
around `[Pasted text #1 +14 lines]` and to any path in parentheses inside a
Hebrew sentence.

The renderer patch fixes this per cell: the row is resolved exactly as the
caret resolves it, the odd-level mirrorable characters are mapped to the
columns they were painted in, and those cells are drawn mirrored. A cell is
only rewritten when it still holds the character the resolution expected
there, so a stale row cannot flip an unrelated glyph. Where more than one
logical text explains the row and they disagree about a bracket, the glyph is
left alone.

Table rows are resolved cell by cell — the frame characters split the row,
because each cell was reordered on its own and the row as a whole is the
reordering of nothing.

This is on by default. `sudo rtl-caret install --no-mirror` leaves brackets
as Claude painted them.

## Install

```sh
npm install -g rtl-caret
sudo rtl-caret install
```

Then quit the editor completely and reopen it.

```sh
rtl-caret status       # what is installed, changes nothing
sudo rtl-caret uninstall
```

`status` reports which parts are live for each patched file, for example
`caret+mirror+align`.

`install` backs up each file it touches to `<file>.rtlbak` before the first
change, is safe to run repeatedly, and refuses to touch a bundle whose shape it
does not recognise. Editor upgrades replace the bundle, so run `install` again
after one.

If your installation lives somewhere unusual:

```sh
sudo rtl-caret install --app /path/to/resources/app
```

If `sudo` reports `node: command not found`, your Node lives outside root's
PATH — common with nvm and fnm. Give it the full path:

```sh
sudo "$(command -v node)" "$(npm root -g)/rtl-caret/bin/rtl-caret.js" install
```

## The terminal font

The caret fix is column arithmetic, so it is only as correct as the cell grid it
maps onto. A terminal font that has no Hebrew glyphs sends every Hebrew
character to a fallback font, and the usual fallbacks (Rubik, Noto Sans Hebrew,
Segoe UI) are proportional: each glyph is painted at its own natural width
instead of one cell. The row then drifts out of the grid, and a caret placed on
the correct *column* still lands between glyphs.

What is needed is a monospace font that covers `U+0590..U+05FF` at exactly the
same advance width as the Latin glyphs. `tools/graft_hebrew.py` builds one, by
grafting the Hebrew block of a donor font onto the monospace font you already
use. The donor outlines are scaled so their alef matches the base's x-height and
re-centred inside the base's fixed advance, so the cell grid is untouched:

```sh
python3 -m venv .venv && .venv/bin/pip install fonttools
.venv/bin/python tools/graft_hebrew.py \
  CascadiaMonoNF-Regular.ttf NotoSansHebrew-Regular.ttf \
  CascadiaHebrew-Regular.ttf "Cascadia Hebrew" Regular
```

The base must be a TrueType (`.ttf`) build - the script rewrites `glyf`
outlines, which a CFF `.otf` does not have.

Build every style the terminal can ask for. A missing face is the same problem
as a missing font: an italic run finds no italic in the grafted family, falls
back to a proportional one, and the narrow letters - `י`, `ו`, `ן` - visibly
stop lining up while the upright text around them is fine. Where there is no
italic base to graft onto, `tools/style_alias.py` re-labels an upright face as
the italic one. Hebrew has no italic form, so the outlines are re-used unchanged
and only the style bits and names differ:

```sh
.venv/bin/python tools/style_alias.py \
  CascadiaHebrew-Regular.ttf CascadiaHebrew-Italic.ttf "Cascadia Hebrew" Italic
.venv/bin/python tools/style_alias.py \
  CascadiaHebrew-Bold.ttf CascadiaHebrew-BoldItalic.ttf "Cascadia Hebrew" "Bold Italic" bold
```

Install all four into `~/.local/share/fonts/` (`fc-cache -f`), then put the
grafted family in the terminal font list, *before* the generic fallback:

```jsonc
"terminal.integrated.fontFamily": "'Cascadia Mono NF', 'Cascadia Hebrew', monospace"
```

Naming the family in that list is what makes it take effect. Without it,
fontconfig picks the fallback by coverage and the grafted font is never
consulted, no matter that it is installed.

To check that a face is grid-correct, every glyph should report one width, and
`fc-match` should return the grafted family for each style rather than a
fallback:

```sh
python3 -c "from fontTools.ttLib import TTFont; f=TTFont('CascadiaHebrew-Italic.ttf'); \
  c=f.getBestCmap(); print({f['hmtx'][c[x]][0] for x in c})"
fc-match "Cascadia Hebrew:italic:lang=he"
```

None of this reorders anything - the bidi reordering is Claude Code's, and the
caret mapping is this patch's. The font's only job is to keep one character in
one cell so both of those stay true on screen.

## Copying Hebrew out of the terminal

The buffer holds what Claude painted, which is visual order, and xterm copies
its cells verbatim. So a copied Hebrew line comes out reversed - and pasting it
back into Claude reorders it a second time, which is why a pasted run lands
mirrored while everything typed around it reads correctly.

`install` patches xterm's own `selectionText` so a selection is handed back as
the text it stands for, recovered the same way the caret recovers it and left
untouched whenever that recovery does not verify. Copy and paste round trip,
and Hebrew copied into any other program arrives readable.

A line that reorders to itself needed no reordering and is copied verbatim,
which is what keeps ordinary Latin lines and mixed lines that already read
correctly out of the way. Where two logical texts repaint identically, the
answer the renderer already resolved for that row is reused rather than guessed
at again.

Turn it off with `--no-copy`. The case for turning it off is Hebrew that was
never reordered in the first place - raw output from a shell command rather than
from Claude - which is drawn in logical order and is copied correctly today.

## Right-aligning RTL rows (opt in)

```sh
sudo rtl-caret install --align
```

Rows whose base direction is Hebrew are flushed to the right edge; rows that
start in Latin script are left alone, so the decision is automatic and per row.

The direction comes from the same resolution the caret uses, sharing one memo
per row. Deciding it separately re-opens the ambiguity described above, and the
row then swings left the moment a Latin character makes a Hebrew-first line look
Latin-first - the alignment flickering back and forth while you type.

The shift is applied to the *source* column each cell is read from, not the
destination, so every column is still written exactly once and no stale cells
are left behind. Rows containing box drawing or block characters are skipped,
because Claude paints frames, separators and progress glyphs with those and
shifting one would tear the layout apart. The caret is shifted by the same
amount.

Known limitation: mouse selection and link hovering use unshifted columns, so on
a right-aligned row they address the wrong cells. Drop `--align` to turn this
off while keeping the caret fix.

## Hebrew dictation (opt in)

Claude Code's CLI has its own `/voice` mode. It records the microphone itself and
streams the audio to whatever `VOICE_STREAM_BASE_URL` points at, so redirecting
that one variable is enough to replace the transcription engine — the mic, the
UI and the keybinding stay Claude's:

```sh
rtl-caret voice setup            # paste your ElevenLabs API key
rtl-caret voice -- claude        # run Claude with dictation redirected here
```

`voice` starts a WebSocket server on `127.0.0.1`, speaks Claude's `voice_stream`
protocol (linear16 16 kHz mono in; `TranscriptInterim` / `TranscriptText` /
`TranscriptEndpoint` / `TranscriptError` out) and transcribes through ElevenLabs
Scribe v2 Realtime with `he`. Anthropic's own backend transcribes Hebrew as
English; this returns Hebrew.

Grey text while you hold the key is a hypothesis, and the committed text arrives
when you let go — that split is Claude's, not ours. What is ours is making sure
the committed text is the accurate engine's and not the fast one's guess: when
the CLI stops the stream it gives the server 1500 ms of silence before giving up,
so the server answers immediately and keeps the socket alive for the full 5000 ms
the client allows. That is the window the engine needs to land its final.

Scribe runs its own voice-activity endpointer, so a finished sentence is
committed while you are still talking rather than at the end of the recording.
When the mic stops, the tail is committed explicitly instead of waiting out the
silence threshold.

Other commands:

```sh
rtl-caret voice serve            # foreground server, prints the export line
rtl-caret voice env              # export line for a server already running
rtl-caret voice status           # is anything reachable, and how is it configured
rtl-caret voice test 5           # record 5s from the mic and print the transcript
```

### Tuning Hebrew

Configuration lives in `~/.config/rtl-caret/voice.json` (mode 0600). The
defaults already aim at dictating Hebrew into a terminal; these are the keys
worth changing:

| key | default | what it does |
| --- | --- | --- |
| `secondaryLanguages` | `["en"]` | other languages allowed inside the same sentence. Terminal Hebrew is code-switched — paths, commands and library names arrive in English — and without this they come back transliterated into Hebrew letters. |
| `keyterms` | `[]` | up to 50 words the model keeps mishearing, each at most 20 characters. Longer ones are dropped rather than truncated. |
| `vadSilenceThresholdSecs` | server's `1.5` | how long a pause ends a sentence. Lower commits sooner while you are still talking; the server accepts 0.3–3.0. |
| `noVerbatim` | `false` | strips fillers and false starts. Off because it also edits speech that was not a filler. |
| `filterBackgroundAudio` | `false` | ignores nearby conversation. Also drops the server's speech threshold from 0.4 to 0.15, so it picks you up more eagerly. |

```json
{
  "language": "he",
  "secondaryLanguages": ["en"],
  "keyterms": ["rtl-caret", "קומיט", "פוש"],
  "vadSilenceThresholdSecs": 0.8
}
```

`--secondary` and `--keyterm` set the same two from the command line, which is
easier while you are still finding out which terms you need:

```sh
rtl-caret voice test 5 --keyterm rtl-caret --keyterm קומיט
```

`rtl-caret voice status` prints the languages and keyterms actually in effect,
including any keyterm dropped for being too long.

`ELEVENLABS_API_KEY` / `XI_API_KEY` override the stored credential. Set `"enabled": false` there to turn dictation off without changing
how you launch Claude — the wrapper then runs the command untouched.

This is a separate feature: `install` and `uninstall` neither enable it nor
depend on it, and nothing about it is injected into the editor. The server binds
loopback only and is unauthenticated, because Claude's client cannot be told to
send a token — any local process can reach it.

### Always on, in every terminal

The wrapper starts the server for one command and stops it afterwards. To have
dictation simply be there — in terminals that are already open, and after a
reboot — run the server as a systemd user service and export the variable from
your shell profile instead.

`~/.config/systemd/user/rtl-caret-voice.service`:

```ini
[Unit]
Description=rtl-caret Hebrew dictation server for Claude Code
After=default.target

[Service]
Type=simple
# Absolute interpreter path: the service starts before any shell runs, so
# nvm/fnm shims are not on PATH yet. Update this after a node upgrade.
ExecStart=/path/to/node /path/to/rtl-caret/bin/rtl-caret.js voice serve
Restart=always
RestartSec=3

[Install]
WantedBy=default.target
```

```sh
systemctl --user enable --now rtl-caret-voice.service
echo 'export VOICE_STREAM_BASE_URL=ws://127.0.0.1:8766' >> ~/.bashrc
```

Pin the same port in `voice.json` so the service and the variable agree, and
pick one the VS Code extension does not already own — it keeps its own
`voice_stream` server on 8765 whether or not its dictation is switched off, so
8766 is the safer default when both are installed. `voice status` names whatever
is listening, which is how you tell the two apart.

Already-open terminals keep the environment they started with; export the
variable by hand there once, or just run `rtl-caret voice -- claude`, which
overrides it for that command either way.

## Scope

Fixes caret placement. It does not change arrow-key direction: `Right` still
means "forward in the text", which in Hebrew moves the caret leftward. That is
ordinary logical cursor movement, and with the caret finally on the right glyph
it behaves predictably.

It also does not address Claude Code re-rendering a transcript left-to-right
after switching screens. That one is in Claude Code itself.

Only the WebGL renderer is patched. If your editor falls back to the DOM
renderer (`"terminal.integrated.gpuAcceleration": "off"`, or no GPU), the patch
has no effect.

## Tests

```sh
npm test
```

Every fixture is a recording of Claude Code driven inside a real pty by the
scripts in `tools/`, not a hand-written string. `typing-samples.json` stores one
entry per keystroke together with the text that produced it, so recovery is
checked against ground truth.

```
painted lines: every logical offset must land on its own glyph
  pass  short hebrew          "שלום עולם "
  pass  hebrew + comma        "שלום, מה נשמע. "
  pass  hebrew + path         "קובץ src/auth.ts שורה 42 "
  pass  wrapped tail          "123 וגם מילה English באמצע. "
  pass  latin base            "hello שלום world"
  pass  typed trailing sp     "שלום,  "

typing samples: 68, recovery failures 0, caret failures 0
edit sequence: 9 steps, 0 failures
alignment stays put: 29 steps, 0 flips
all checks pass  (273 checks)
```

## Re-recording the fixtures

Requires Python with `pyte`:

```sh
python3 -m venv .venv && .venv/bin/pip install pyte
.venv/bin/python tools/probe4.py     # typing samples
```

`probe.py` captures a single line, `probe2.py` a long wrapping sentence,
`probe3.py` punctuation typed one character at a time, `probe4.py` mixed
Hebrew/English with the typed text recorded alongside.

## License

MIT

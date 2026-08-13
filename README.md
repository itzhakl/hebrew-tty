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

`status` reports `caret` or `caret+align` for each patched file.

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

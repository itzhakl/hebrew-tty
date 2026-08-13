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
all checks pass  (229 checks)
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

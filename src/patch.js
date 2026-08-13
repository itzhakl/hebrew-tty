'use strict';

const fs = require('fs');
const path = require('path');

const MARKER = '/*rtl-caret*/';

// <var>=Math.min(this._terminal.buffer.active.cursorX,<term>.cols-1)
const ANCHOR = /(\w+)=Math\.min\(this\._terminal\.buffer\.active\.cursorX,(\w+)\.cols-1\)/;

// The per-column loop that fills the render model for one row. Shifting the
// source column here right-aligns the row without leaving stale cells behind.
const ROW_ANCHOR =
  /(\w+)=this\._characterJoinerService\.getJoinedCharacters\((\w+)\),(\w+)=0;\3<(\w+)\.cols;\3\+\+\)\{if\((\w+)=this\._cellColorResolver\.result\.bg,(\w+)\.loadCell\(\3,(\w+)\)/;

const ALIGN_FLAG = 'globalThis.__rtlAlign=true;';
const MIRROR_FLAG = 'globalThis.__rtlMirrorGlyphs=true;';

// Where each editor keeps the WebGL renderer addon.
const APP_ROOTS = [
  '/usr/share/codium/resources/app',
  '/usr/share/vscodium/resources/app',
  '/usr/share/code/resources/app',
  '/usr/share/code-insiders/resources/app',
  '/usr/lib/code/resources/app',
  '/opt/visual-studio-code/resources/app',
  '/opt/vscodium/resources/app',
  '/Applications/VSCodium.app/Contents/Resources/app',
  '/Applications/Visual Studio Code.app/Contents/Resources/app',
  path.join(process.env.HOME || '', '.vscode-server/bin')
];

const ADDON_REL = 'node_modules/@xterm/addon-webgl/lib';
const ADDON_FILES = ['addon-webgl.js', 'addon-webgl.mjs'];

function discover(extraRoots = []) {
  const targets = [];
  for (const root of [...extraRoots, ...APP_ROOTS]) {
    if (!root) continue;
    const dir = path.join(root, ADDON_REL);
    if (!fs.existsSync(dir)) continue;
    for (const name of ADDON_FILES) {
      const file = path.join(dir, name);
      if (fs.existsSync(file)) targets.push(file);
    }
  }
  return targets;
}

function backupOf(file) {
  return `${file}.rtlbak`;
}

function stateOf(file) {
  if (!fs.existsSync(file)) return 'missing';
  const src = fs.readFileSync(file, 'utf8');
  if (src.includes(MARKER)) return 'patched';
  if (ANCHOR.test(src)) return 'unpatched';
  return 'no-anchor';
}

/* bidi-js ships UMD. Shadow module/exports/define so the CommonJS branch is
 * taken deterministically rather than registering with the host's AMD loader. */
function buildPayload({ align = false, mirror = false } = {}) {
  const bidi = fs.readFileSync(require.resolve('bidi-js/dist/bidi.min.js'), 'utf8');
  const caret = fs.readFileSync(path.join(__dirname, 'caret.js'), 'utf8');
  return [
    `${MARKER}if(!globalThis.__rtlCaret){`,
    '(function(){var module={exports:{}},exports=module.exports,define=void 0;',
    bidi,
    'try{globalThis.__rtlBidi=module.exports();}catch(e){}',
    '})();',
    caret,
    align ? ALIGN_FLAG : '',
    mirror ? MIRROR_FLAG : '',
    '}',
    ''
  ].join('\n');
}

function writeAtomic(file, text) {
  const tmp = `${file}.rtltmp`;
  const mode = fs.statSync(file).mode;
  fs.writeFileSync(tmp, text, 'utf8');
  fs.chmodSync(tmp, mode);
  fs.renameSync(tmp, file);
}

function applyTo(file, { align = false, mirror = true } = {}) {
  let state = stateOf(file);
  if (state === 'missing') return { file, ok: false, note: 'missing' };

  if (state === 'patched') {
    if (!fs.existsSync(backupOf(file))) {
      return { file, ok: false, note: 'patched but no backup, refusing to re-apply' };
    }
    fs.copyFileSync(backupOf(file), file);
    state = stateOf(file);
  }
  if (state === 'no-anchor') {
    return { file, ok: false, note: 'anchor not found, left untouched' };
  }

  if (!fs.existsSync(backupOf(file))) fs.copyFileSync(file, backupOf(file));

  let src = fs.readFileSync(file, 'utf8');
  let note = 'caret';

  if (align || mirror) {
    const r = ROW_ANCHOR.exec(src);
    if (!r) {
      note = 'caret (row anchor not found, mirroring and alignment skipped)';
      align = false;
      mirror = false;
    } else {
      const [, joined, row, x, term, bg, line, cell] = r;
      const rewritten =
        `${joined}=this._characterJoinerService.getJoinedCharacters(${row}),` +
        `globalThis.__rtlRow(${line},${term}.cols,${row}-${term}.buffer.ydisp),` +
        `${x}=0;${x}<${term}.cols;${x}++){` +
        `if(${bg}=this._cellColorResolver.result.bg,` +
        `${line}.loadCell(globalThis.__rtlSrc(${x},${term}.cols),${cell}),` +
        `globalThis.__rtlMirror(globalThis.__rtlSrc(${x},${term}.cols),${cell})`;
      src = src.slice(0, r.index) + rewritten + src.slice(r.index + r[0].length);
      note = ['caret', mirror && 'mirroring', align && 'alignment'].filter(Boolean).join(' + ');
    }
  }

  const m = ANCHOR.exec(src);
  const call =
    `${m[1]}=globalThis.__rtlCaret(this._terminal,` +
    `Math.min(this._terminal.buffer.active.cursorX,${m[2]}.cols-1))`;
  const patched =
    buildPayload({ align, mirror }) +
    src.slice(0, m.index) + call + src.slice(m.index + m[0].length);
  writeAtomic(file, patched);
  return { file, ok: true, note };
}

function revertFile(file) {
  const bak = backupOf(file);
  if (!fs.existsSync(bak)) return { file, ok: false, note: 'no backup' };
  fs.copyFileSync(bak, file);
  return { file, ok: true, note: 'reverted' };
}

module.exports = {
  MARKER, ANCHOR, ROW_ANCHOR, ALIGN_FLAG, MIRROR_FLAG,
  discover, stateOf, applyTo, revertFile, buildPayload, backupOf
};

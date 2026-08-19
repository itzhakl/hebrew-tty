'use strict';

const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const MARKER = '/*rtl-caret*/';

/* An editor keeps running the payload it was patched with, so "patched" alone
 * says nothing about which version is live. The stamp identifies the injected
 * code, and status compares it against this checkout. */
const STAMP = /\/\*rtl-caret:([0-9a-f]{12})\*\//;

function stampFor({ align = false, mirror = false, copy = false } = {}) {
  const caret = fs.readFileSync(path.join(__dirname, 'caret.js'));
  return crypto
    .createHash('sha256')
    .update(caret)
    .update(`|align=${align}|mirror=${mirror}|copy=${copy}`)
    .digest('hex')
    .slice(0, 12);
}

// <var>=Math.min(this._terminal.buffer.active.cursorX,<term>.cols-1)
const ANCHOR = /(\w+)=Math\.min\(this\._terminal\.buffer\.active\.cursorX,(\w+)\.cols-1\)/;

// The per-column loop that fills the render model for one row. Shifting the
// source column here right-aligns the row without leaving stale cells behind.
const ROW_ANCHOR =
  /(\w+)=this\._characterJoinerService\.getJoinedCharacters\((\w+)\),(\w+)=0;\3<(\w+)\.cols;\3\+\+\)\{if\((\w+)=this\._cellColorResolver\.result\.bg,(\w+)\.loadCell\(\3,(\w+)\)/;

/* The end of xterm's selectionText getter, where the selected rows are still a
 * list of lines. Wrapping the list rather than the joined string keeps one
 * entry per painted row, which is the unit the recovery works on. Both the CJS
 * and the ESM build minify the tail differently, so the anchor stops before
 * the line separator. */
const COPY_ANCHOR = /return (\w+)\.map\((\w+)=>\2\.replace\((\w+)," "\)\)\.join\(/;

const ALIGN_FLAG = 'globalThis.__rtlAlign=true;';
const MIRROR_FLAG = 'globalThis.__rtlMirrorGlyphs=true;';
const COPY_FLAG = 'globalThis.__rtlCopyLogical=true;';

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

// The caret and the row rewrite live in the renderer; copying is xterm's core.
const CORE_REL = 'node_modules/@xterm/xterm/lib';
const CORE_FILES = ['xterm.js', 'xterm.mjs'];

function kindOf(file) {
  return file.includes('addon-webgl') ? 'webgl' : 'core';
}

function discover(extraRoots = []) {
  const targets = [];
  for (const root of [...extraRoots, ...APP_ROOTS]) {
    if (!root) continue;
    for (const [rel, names] of [[ADDON_REL, ADDON_FILES], [CORE_REL, CORE_FILES]]) {
      const dir = path.join(root, rel);
      if (!fs.existsSync(dir)) continue;
      for (const name of names) {
        const file = path.join(dir, name);
        if (fs.existsSync(file)) targets.push(file);
      }
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
  if ((kindOf(file) === 'webgl' ? ANCHOR : COPY_ANCHOR).test(src)) return 'unpatched';
  return 'no-anchor';
}

/* Which payload is in the file, and whether it is the one this checkout
 * builds. An editor upgrade or an older install both show up here. */
function versionOf(file) {
  if (stateOf(file) !== 'patched') return null;
  const src = fs.readFileSync(file, 'utf8');
  const m = STAMP.exec(src);
  const align = src.includes(ALIGN_FLAG);
  const mirror = src.includes(MIRROR_FLAG);
  const copy = src.includes(COPY_FLAG);
  return {
    align,
    mirror,
    copy,
    kind: kindOf(file),
    stamp: m ? m[1] : null,
    current: !!m && m[1] === stampFor({ align, mirror, copy })
  };
}

/* bidi-js ships UMD. Shadow module/exports/define so the CommonJS branch is
 * taken deterministically rather than registering with the host's AMD loader. */
function buildPayload({ align = false, mirror = false, copy = false } = {}) {
  const bidi = fs.readFileSync(require.resolve('bidi-js/dist/bidi.min.js'), 'utf8');
  const caret = fs.readFileSync(path.join(__dirname, 'caret.js'), 'utf8');
  return [
    `${MARKER}/*rtl-caret:${stampFor({ align, mirror, copy })}*/if(!globalThis.__rtlCaret){`,
    '(function(){var module={exports:{}},exports=module.exports,define=void 0;',
    bidi,
    'try{globalThis.__rtlBidi=module.exports();}catch(e){}',
    '})();',
    caret,
    align ? ALIGN_FLAG : '',
    mirror ? MIRROR_FLAG : '',
    copy ? COPY_FLAG : '',
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

function applyTo(file, { align = false, mirror = true, copy = true } = {}) {
  let state = stateOf(file);
  if (state === 'missing') return { file, ok: false, note: 'missing' };

  if (state === 'patched') {
    if (!fs.existsSync(backupOf(file))) {
      return { file, ok: false, note: 'patched but no backup, refusing to re-apply' };
    }
    fs.copyFileSync(backupOf(file), file);
    state = stateOf(file);
  }
  // Turning an option off has to leave the file it drives reverted, which the
  // restore above already did.
  if (kindOf(file) === 'core' && !copy) {
    return { file, ok: true, note: 'copy off, left unpatched' };
  }
  if (state === 'no-anchor') {
    return { file, ok: false, note: 'anchor not found, left untouched' };
  }

  if (!fs.existsSync(backupOf(file))) fs.copyFileSync(file, backupOf(file));

  let src = fs.readFileSync(file, 'utf8');
  let note = 'caret';

  if (kindOf(file) === 'core') {
    const r = COPY_ANCHOR.exec(src);
    if (!r) return { file, ok: false, note: 'copy anchor not found, left untouched' };
    const [, lines, line, ws] = r;
    const rewritten =
      `return globalThis.__rtlCopy(${lines}.map(${line}=>${line}.replace(${ws}," "))).join(`;
    src = src.slice(0, r.index) + rewritten + src.slice(r.index + r[0].length);
    writeAtomic(file, buildPayload({ align, mirror, copy }) + src);
    return { file, ok: true, note: 'copy' };
  }

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
        `globalThis.__rtlRow(${line},${term},${row}-${term}.buffer.ydisp),` +
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
    buildPayload({ align, mirror, copy }) +
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
  MARKER, ANCHOR, ROW_ANCHOR, COPY_ANCHOR, ALIGN_FLAG, MIRROR_FLAG, COPY_FLAG, STAMP,
  discover, kindOf, stateOf, versionOf, stampFor, applyTo, revertFile, buildPayload, backupOf
};

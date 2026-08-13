#!/usr/bin/env node
/* Run recorded frames through the payload that is actually installed in the
 * editor bundle, rather than through src/caret.js. Catches a stale install and
 * anything the embedding step mangles on the way in.
 *
 *   node tools/verify_installed.js [frames.json]
 */
'use strict';

const fs = require('fs');
const path = require('path');
const vm = require('vm');
const { discover, backupOf, MARKER } = require('../src/patch');

const COLS = 100;

function payloadOf(file) {
  const patched = fs.readFileSync(file, 'utf8');
  if (!patched.startsWith(MARKER)) throw new Error(`not patched: ${file}`);
  const original = fs.readFileSync(backupOf(file), 'utf8');
  const head = original.slice(0, 120);
  const at = patched.indexOf(head);
  if (at < 0) throw new Error(`cannot find the original bundle inside ${file}`);
  return patched.slice(0, at);
}

function fakeLine(text) {
  return { translateToString: () => text.padEnd(COLS, ' ') };
}

function fakeTerm(g, text, cursorY, cursorX) {
  return {
    cols: COLS,
    buffer: { active: { baseY: 0, cursorX, cursorY, getLine: () => fakeLine(text) } }
  };
}

const frames = JSON.parse(
  fs.readFileSync(process.argv[2] || path.join(__dirname, 'bulk.json'), 'utf8')
);

for (const file of discover()) {
  const g = { console };
  g.globalThis = g;
  vm.createContext(g);
  vm.runInContext(payloadOf(file), g, { filename: file });

  const missing = ['__rtlBidi', '__rtlCaret', '__rtlRow', '__rtlSrc', '__rtlLog']
    .filter((k) => g[k] === undefined);
  if (missing.length) {
    console.log(`FAIL ${file}\n  missing: ${missing.join(', ')}`);
    continue;
  }

  let bad = 0;
  for (const e of frames) {
    const shift = g.__rtlRow(fakeLine(e.row), COLS, e.y);
    const caret = g.__rtlCaret(fakeTerm(g, e.row, e.y, e.caret), e.caret);
    let drawn = '';
    for (let x = 0; x < COLS; x++) drawn += e.row.padEnd(COLS, ' ')[g.__rtlSrc(x, COLS)];
    if (drawn.trim() !== e.row.trim() || caret < 0 || caret >= COLS) {
      bad++;
      console.log(`  bad frame ${JSON.stringify(e.typed)}\n    ${JSON.stringify(drawn)}`);
    }
  }
  console.log(
    `${bad ? 'FAIL' : 'ok  '} ${path.basename(file)}  ` +
      `frames ${frames.length}, bad ${bad}, log entries ${g.__rtlLog.length}` +
      `${g.__rtlMirror ? ', mirror hook present' : ''}`
  );
  if (!bad) {
    const last = g.__rtlLog[g.__rtlLog.length - 1];
    console.log(`     last log entry: ${JSON.stringify(last)}`);
  }
}

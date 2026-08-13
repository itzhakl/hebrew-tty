#!/usr/bin/env node
'use strict';

/* Both fixtures are recordings of Claude Code running in a real pty, captured
 * by tools/probe*.py. painted-lines.json holds finished lines; typing-samples
 * holds one entry per keystroke together with the text that produced it, which
 * makes the recovery step checkable against ground truth rather than a guess. */

const path = require('path');
globalThis.__rtlBidi = require('bidi-js')();
const M = require(path.join(__dirname, '..', 'src', 'caret.js'));

const painted = require('./fixtures/painted-lines.json');
const typing = require('./fixtures/typing-samples.json');

let failures = 0;
let checks = 0;

function fail(msg) {
  failures++;
  if (failures <= 12) console.log(`  FAIL ${msg}`);
}

function term(line, cursorY = 24) {
  return {
    buffer: {
      active: {
        baseY: 0,
        cursorY,
        getLine: () => ({ translateToString: () => line })
      }
    }
  };
}

console.log('painted lines: every logical offset must land on its own glyph');
for (const { name, row } of painted) {
  const { a, e } = M.spanOf(row);
  const rec = M.recover(row.slice(a, e + 1));
  if (!rec) {
    checks++;
    fail(`${name}: no logical text recovered`);
    continue;
  }
  let bad = 0;
  for (let i = 0; i < rec.text.length; i++) {
    checks++;
    const v = M.mapCaret(term(row), a + i);
    if (row[v] !== rec.text[i]) {
      bad++;
      fail(`${name} i=${i} want ${JSON.stringify(rec.text[i])} got ${JSON.stringify(row[v])}`);
    }
  }
  console.log(`  ${bad ? 'FAIL' : 'pass'}  ${name}  ${JSON.stringify(rec.text)}`);
}

console.log('\nlines without Hebrew must never move');
for (const line of ['❯  npm run build', '  const x = 42;']) {
  checks++;
  const v = M.mapCaret(term(line), 7);
  if (v !== 7) fail(`latin-only line moved to ${v}`);
}
console.log('  pass');

console.log('\ntyping samples: recovery must equal what was actually typed');
let recBad = 0;
let caretBad = 0;
for (const { typed, row, caret } of typing) {
  const { a, e } = M.spanOf(row);
  // Drive the per-row memo the same way live typing does.
  M.mapCaret(term(row), caret);
  const rec = M.recover(row.slice(a, e + 1), 24);
  const got = rec ? rec.text : null;
  checks++;
  // The input box carries one trailing pad cell, so content is typed + ' '.
  if (got !== typed && got !== `${typed} ` && got !== typed.replace(/\s+$/, '')) {
    recBad++;
    fail(`typed ${JSON.stringify(typed)} recovered ${JSON.stringify(got)}`);
    continue;
  }
  const i = typed.length - 1;
  if (typed[i] === ' ') continue; // a trailing space has no painted cell to land on
  checks++;
  const v = M.mapCaret(term(row), a + i);
  if (row[v] !== typed[i]) {
    caretBad++;
    fail(`typed ${JSON.stringify(typed)} caret ${a + i} -> ${v} (${JSON.stringify(row[v])})`);
  }
}
console.log(`  samples ${typing.length}, recovery failures ${recBad}, caret failures ${caretBad}`);

console.log(`\n${failures ? `${failures} FAILURES` : 'all checks pass'}  (${checks} checks)`);
process.exit(failures ? 1 : 0);

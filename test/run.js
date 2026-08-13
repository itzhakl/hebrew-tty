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

/* The caret follows the character just typed, so for a caret at logical offset
 * i the neighbouring character is i-1. A line cursor draws on the left edge of
 * its cell, so that neighbour occupies the caret's own cell when it is RTL and
 * the cell to the left when it is LTR. */
function neighbourGlyph(row, cell, rtl) {
  return rtl ? row[cell] : row[cell - 1];
}

console.log('painted lines: the caret must sit against the character it follows');
for (const { name, row } of painted) {
  const { a, e } = M.spanOf(row);
  const rec = M.recover(row.slice(a, e + 1));
  if (!rec) {
    checks++;
    fail(`${name}: no logical text recovered`);
    continue;
  }
  let bad = 0;
  for (let i = 1; i < rec.text.length; i++) {
    checks++;
    const v = M.mapCaret(term(row), a + i);
    const rtl = (rec.levels.levels[i - 1] & 1) === 1;
    const got = neighbourGlyph(row, v, rtl);
    if (got !== rec.text[i - 1]) {
      bad++;
      fail(`${name} i=${i} want ${JSON.stringify(rec.text[i - 1])} got ${JSON.stringify(got)}`);
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
  // After typing, the caret must be adjacent to the last character typed, on
  // whichever side is "forward" for that character's direction.
  const i = typed.length - 1;
  if (typed[i] === ' ') continue; // a trailing space has no painted cell
  checks++;
  const u = a + rec.order.indexOf(i);
  const rtl = (rec.levels.levels[i] & 1) === 1;
  const want = rtl ? u : u + 1;
  const v = M.mapCaret(term(row), caret);
  if (v !== want) {
    caretBad++;
    fail(`typed ${JSON.stringify(typed)} caret ${caret} -> ${v}, expected ${want}`);
  }
}
console.log(`  samples ${typing.length}, recovery failures ${recBad}, caret failures ${caretBad}`);

const editSeq = require('./fixtures/edit-sequence.json');

console.log('\nedit sequence: caret stays against the text through deletes and arrows');
let editBad = 0;
for (const { action, row, caret } of editSeq) {
  const { a, e } = M.spanOf(row);
  const rec = M.recover(row.slice(a, e + 1), 24);
  checks++;
  if (!rec) {
    editBad++;
    fail(`${action}: no logical text recovered`);
    continue;
  }
  const d = caret - a;
  if (d <= 0 || d > rec.text.length) continue;
  const v = M.mapCaret(term(row), caret);
  const rtl = (rec.levels.levels[d - 1] & 1) === 1;
  const got = neighbourGlyph(row, v, rtl);
  if (got !== rec.text[d - 1]) {
    editBad++;
    fail(`${action}: caret ${caret} -> ${v}, next to ${JSON.stringify(got)} not ${JSON.stringify(rec.text[d - 1])}`);
  }
}
console.log(`  steps ${editSeq.length}, failures ${editBad}`);

console.log(`\n${failures ? `${failures} FAILURES` : 'all checks pass'}  (${checks} checks)`);
process.exit(failures ? 1 : 0);

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

console.log('\nalignment: only plain RTL rows shift, and every column stays covered');
const COLS = 100;
const alignCases = [
  ['hebrew input row', '❯  םלוע םולש', true],
  ['hebrew with path', '❯  42 הרוש src/auth.ts ץבוק', true],
  ['latin base row', '❯ hello םולש world', false],
  ['no hebrew at all', '❯  npm run build', false],
  ['box drawing frame', '│  םולש                    │', false],
  ['separator rule', '─'.repeat(60), false]
];
for (const [name, row, shouldShift] of alignCases) {
  checks++;
  const shift = M.computeShift(row, COLS);
  if (shouldShift ? !(shift > 0) : shift !== 0) {
    fail(`alignment ${name}: shift ${shift}, expected ${shouldShift ? '> 0' : '0'}`);
    continue;
  }
  if (!shift) continue;

  // Shifting must land the last glyph on the final column.
  let end = row.length - 1;
  while (end >= 0 && /\s/.test(row[end])) end--;
  checks++;
  if (end + shift !== COLS - 1) {
    fail(`alignment ${name}: last glyph lands at ${end + shift}, not ${COLS - 1}`);
  }

  // Every destination column must read a source column, and the columns that
  // fall off the left must read from the blank tail rather than out of range.
  M.setShift(shift);
  const sources = new Set();
  let bad = 0;
  for (let x = 0; x < COLS; x++) {
    const src = M.sourceColumn(x, COLS);
    if (src < 0 || src >= COLS) bad++;
    if (x >= shift) sources.add(src);
  }
  M.setShift(0);
  checks++;
  if (bad) fail(`alignment ${name}: ${bad} source columns out of range`);
  checks++;
  if (sources.size !== COLS - shift) {
    fail(`alignment ${name}: ${sources.size} distinct sources, expected ${COLS - shift}`);
  }
}
console.log(`  cases ${alignCases.length}`);

console.log('\nalignment stays put: a Latin word must not flip a Hebrew line left');
{
  // Replayed in order, exactly as typed, so the per-row memo is exercised.
  const seq = typing.filter(s => s.typed.startsWith('שלום'));
  const ROW = 24;
  let flips = 0;
  let firstRtlSeen = false;
  for (const { typed, row, caret } of seq) {
    M.mapCaret(term(row, ROW), caret);           // resolve the row like typing does
    const shift = M.computeShift(row, COLS, ROW);
    checks++;
    if (shift > 0) {
      firstRtlSeen = true;
    } else if (firstRtlSeen) {
      flips++;
      fail(`alignment flipped left at ${JSON.stringify(typed)}`);
    }
  }
  console.log(`  steps ${seq.length}, flips ${flips}`);
}

/* Mirroring: bidi rule L4 says a mirrorable character at an odd level is drawn
 * as its mirror. Claude paints the reordering without it, so bidi-js's own
 * getReorderedString - which does apply L4 - is the reference to match. */
console.log('\nmirroring: brackets in an RTL run must be drawn mirrored');
{
  const bidi = globalThis.__rtlBidi;
  const paint = (s, mirrored) => {
    const levels = bidi.getEmbeddingLevels(s, 'auto');
    if (mirrored) return bidi.getReorderedString(s, levels, 0, s.length - 1);
    return bidi.getReorderedIndices(s, levels, 0, s.length - 1).map(i => s[i]).join('');
  };
  const draw = row => {
    const map = M.rowMirrors(row);
    if (!map) return row;
    M.setMirrors(map);
    const out = row.split('');
    for (const x of Object.keys(map)) {
      const cell = { content: (1 << 22) | out[x].charCodeAt(0) };
      M.mirrorCell(Number(x), cell);
      out[x] = String.fromCharCode(cell.content & 0x1fffff);
    }
    M.setMirrors(null);
    return out.join('');
  };

  const lines = [
    'הכתבה בפרצים (צאנק, הפסקה, צאנק)',
    'הדבקה של 15 שורות [Pasted text #1 +14 lines] (פלייסהולדר)',
    'העברתי את 12 הפריימים דרך הקוד (tools/replay.js): מיפוי',
    'ראה [docs](https://x.dev) ואז שלום',
    'שלום עולם',
    'const x = foo(bar);'
  ];
  for (const logical of lines) {
    checks++;
    const got = draw(paint(logical, false));
    const want = paint(logical, true);
    if (got !== want) fail(`mirroring ${JSON.stringify(logical)} -> ${JSON.stringify(got)}`);
  }

  // A table row is several independently reordered cells, so each cell has to
  // be resolved on its own or none of them resolves at all.
  checks++;
  const cells = ['(בדיקה) מקרה', 'טקסט [Pasted text] כאן'];
  const row = `│  ${paint(cells[0], false)}  │  ${paint(cells[1], false)}  │`;
  const wantRow = `│  ${paint(cells[0], true)}  │  ${paint(cells[1], true)}  │`;
  if (draw(row) !== wantRow) fail(`mirroring table row -> ${JSON.stringify(draw(row))}`);

  // The prompt glyph is mirrorable and is painted outside the reordered span.
  checks++;
  const prompt = `❯  ${paint('שלום (בדיקה)', false)}`;
  if (draw(prompt)[0] !== '❯') fail('mirroring flipped the prompt glyph');

  // Nothing without Hebrew may be touched, whatever brackets it holds.
  for (const line of ['const x = foo(bar);', '  if (a[0] < b[1]) {']) {
    checks++;
    if (draw(line) !== line) fail(`mirroring changed a latin line: ${line}`);
  }
  console.log(`  cases ${lines.length + 5}`);
}

/* Copy: the buffer holds painted order, so copying a row verbatim and pasting
 * it back reorders it a second time. Every recorded row must come back as the
 * text that produced it, which is exactly what makes the round trip hold. */
console.log('\ncopy: a painted row must come back as the text it stands for');
{
  /* The per-row memo is the tie-break, and the checks above have already
   * replayed these rows several times over. A fresh module gives the copy
   * checks the same starting point a new terminal has. */
  const caretPath = require.resolve(path.join(__dirname, '..', 'src', 'caret.js'));
  delete require.cache[caretPath];
  const C = require(caretPath);

  let copyBad = 0;
  for (const { typed, row, caret } of typing) {
    const { a, e } = C.spanOf(row);
    if (e < a) continue;
    // A row is painted before it can be selected, and painting is what breaks
    // the tie between the logical texts that repaint identically.
    C.mapCaret(term(row), caret);
    checks++;
    const out = C.logicalLine(row);
    // Recovery is a permutation, so the span keeps its place and its length.
    const got = out.slice(a, e + 1);
    if (got !== typed && got !== `${typed} ` && got !== typed.replace(/\s+$/, '')) {
      copyBad++;
      fail(`copy ${JSON.stringify(typed)} came back ${JSON.stringify(got)}`);
      continue;
    }
    checks++;
    if (out.slice(0, a) !== row.slice(0, a) || out.slice(e + 1) !== row.slice(e + 1)) {
      copyBad++;
      fail(`copy ${JSON.stringify(typed)} disturbed the prompt or the padding`);
    }
  }
  console.log(`  samples ${typing.length}, failures ${copyBad}`);

  // Anything the recovery cannot verify, and anything without RTL, is handed
  // back untouched rather than guessed at.
  for (const line of ['❯  npm run build', '  const x = foo(bar);', '', '   ']) {
    checks++;
    if (C.logicalLine(line) !== line) fail(`copy changed a non-RTL line: ${JSON.stringify(line)}`);
  }
  console.log('  non-RTL lines untouched');
}

/* A vertical split means one buffer row carries both panes and the rule
 * between them. Recorded by tools/probe7.py from tmux running two real Claude
 * sessions side by side. */
{
  console.log('\nsplit panes: a row that holds two panes must be cut at the divider');
  const split = require('./fixtures/tmux-split.json');
  const rows = split.lines;
  const width = split.cols;

  const dividers = M.dividersFromRows(rows, width);
  checks++;
  if (dividers.length !== 1 || dividers[0] !== 100) {
    fail(`divider columns ${JSON.stringify(dividers)}, expected [100]`);
  }

  // A table border is a rule too, but it does not run the height of the
  // screen. Only a divider does, which is the whole basis for telling them
  // apart, so a short one must not be picked up.
  const shortRule = rows.map((line, y) =>
    y < 8 ? line : line.slice(0, 100) + ' ' + line.slice(101));
  checks++;
  if (M.dividersFromRows(shortRule, width).length !== 0) {
    fail('a rule on a few rows was taken for a pane divider');
  }

  const mixed = rows.find((line) => /[֐-׿]/.test(line.slice(0, 100)) &&
                                    /[֐-׿]/.test(line.slice(101)));
  checks++;
  if (!mixed) fail('the recording holds no row with Hebrew in both panes');

  if (mixed) {
    const segs = M.segmentsOf(dividers, width);
    const term = {
      cols: width,
      rows: rows.length,
      buffer: {
        ydisp: 0,
        active: {
          baseY: 0,
          cursorY: rows.indexOf(mixed),
          viewportY: 0,
          getLine: (y) => (rows[y] === undefined ? null : {
            translateToString: (trim) => (trim ? rows[y].replace(/\s+$/, '') : rows[y])
          })
        }
      }
    };

    // The caret of the left pane must be placed by the left pane's text. Before
    // the row was cut, the span ran across the divider and the arithmetic came
    // from the other pane.
    const left = segs[0];
    for (let c = 2; c < 26; c++) {
      checks++;
      const got = M.mapCaret(term, c);
      if (got < left.a || got > left.b) {
        fail(`caret at column ${c} of the left pane mapped to ${got}, outside [${left.a},${left.b}]`);
      }
    }

    // Alignment flushes each pane to its own right edge, and never writes a
    // column belonging to the other pane or to the divider itself.
    M.rowShift({ translateToString: () => mixed }, term, width, term.buffer.active.cursorY);
    for (const seg of segs) {
      const body = mixed.slice(seg.a, seg.b + 1);
      let end = body.length - 1;
      while (end >= 0 && /\s/.test(body[end])) end--;
      const shift = M.computeShift(body, seg.b - seg.a + 1);
      checks++;
      if (!(shift > 0)) {
        fail(`pane at ${seg.a}: shift ${shift}, expected > 0`);
      } else if (end + shift !== seg.b - seg.a) {
        fail(`pane at ${seg.a}: last glyph lands at ${end + shift}, not ${seg.b - seg.a}`);
      }
    }
    // The renderer is handed the core buffer, which has no getLine at all.
    // Reading it through the wrong API would silently switch the split off.
    M.forgetDividers();
    const coreTerm = {
      cols: width,
      rows: rows.length,
      buffer: {
        ydisp: 0,
        lines: {
          get: (y) => (rows[y] === undefined ? null : {
            translateToString: (trim) => (trim ? rows[y].replace(/\s+$/, '') : rows[y])
          })
        }
      }
    };
    checks++;
    if (M.rowShift({ translateToString: () => mixed }, coreTerm, width, 34) !== 75) {
      fail('the core buffer shape did not resolve the divider');
    }
    M.forgetDividers();
    M.rowShift({ translateToString: () => mixed }, term, width, term.buffer.active.cursorY);

    checks++;
    if (M.sourceColumn(100, width) !== 100) fail('the divider column was moved');
    for (const seg of segs) {
      for (let x = seg.a; x <= seg.b; x++) {
        const src = M.sourceColumn(x, width);
        checks++;
        if (src < seg.a || src > seg.b) {
          fail(`column ${x} of the pane at ${seg.a} read from ${src}, outside its own pane`);
        }
      }
    }
    console.log(`  divider at ${dividers[0]}, panes ${JSON.stringify(segs.map((s) => [s.a, s.b]))}`);
  }

  // A second multiplexer, recorded the same way. The rule is found by height,
  // not by who drew it, so nothing here is specific to tmux.
  {
    const herdr = require('./fixtures/herdr-split.json');
    const found = M.dividersFromRows(herdr.lines, herdr.cols);
    checks++;
    if (found.length !== 1 || found[0] !== 25) {
      fail(`herdr divider columns ${JSON.stringify(found)}, expected [25]`);
    }
    console.log(`  herdr divider at ${found[0]}`);
  }
}

console.log(`\n${failures ? `${failures} FAILURES` : 'all checks pass'}  (${checks} checks)`);
process.exit(failures ? 1 : 0);

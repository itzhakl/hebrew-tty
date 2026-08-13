#!/usr/bin/env node
/* Replay recorded frames through the patch: report the caret it produces and
 * the row the renderer would draw once the source columns are shifted.
 *
 *   node tools/replay.js tools/bulk.json
 */
'use strict';

const fs = require('fs');
const path = require('path');
const bidiFactory = require('bidi-js');

globalThis.__rtlBidi = bidiFactory();
const caret = require(path.join(__dirname, '..', 'src', 'caret.js'));

const COLS = 100;

function fakeTerm(rowText, cursorY, cursorX) {
  const padded = rowText.padEnd(COLS, ' ');
  return {
    cols: COLS,
    buffer: {
      active: {
        baseY: 0,
        cursorX,
        cursorY,
        getLine: () => ({ translateToString: () => padded })
      }
    }
  };
}

function drawn(rowText, shift) {
  const padded = rowText.padEnd(COLS, ' ');
  caret.setShift(shift);
  let out = '';
  for (let x = 0; x < COLS; x++) out += padded[caret.sourceColumn(x, COLS)];
  caret.setShift(0);
  return out.replace(/ +$/, '');
}

const file = process.argv[2] || path.join(__dirname, 'bulk.json');
for (const e of JSON.parse(fs.readFileSync(file, 'utf8'))) {
  const rowText = e.row.padEnd(COLS, ' ');
  const mapped = caret.mapCaret(fakeTerm(rowText, e.y, e.caret), e.caret);
  const shift = caret.computeShift(rowText, COLS, e.y);
  console.log(`${e.kind}  typed=${JSON.stringify(e.typed)}`);
  console.log(`  painted : ${JSON.stringify(e.row)}`);
  console.log(`  drawn   : ${JSON.stringify(drawn(rowText, shift))}`);
  console.log(`  caret ${e.caret} -> ${mapped}${shift ? ` (+shift ${shift} = ${mapped + shift})` : ''}`);
}

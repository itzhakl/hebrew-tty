/* rtl-caret: put the terminal caret on the glyph it is actually editing.
 *
 * Claude Code reorders each painted line with bidi-js (base direction "auto")
 * but reports the caret at a logical offset, so on any line containing Hebrew
 * the caret lands on the wrong glyph. Running the same library here reproduces
 * the exact permutation Claude used, so the caret is mapped rather than guessed.
 *
 * Only the painted line is available, so the logical text is recovered by
 * iterating unpermute(painted) to a fixpoint and then verified by reordering it
 * and requiring the result to equal the painted line. Nothing is moved unless
 * that check passes.
 *
 * Expects globalThis.__rtlBidi to be an initialised bidi-js instance.
 */
(function () {
  'use strict';

  // Escapes, not literals: this file gets embedded into other files, and a
  // literal no-break space silently normalises to a plain space on the way.
  var RTL = /[֐-ࣿיִ-﷿ﹰ-﻿]/;
  var WS = /[ \t  - 　]/;

  // The prompt glyph and the wrap indent sit outside the reordered span, and
  // both are two columns wide. Anything wider eats the input box's trailing pad
  // cell, which RTL paints at the left edge exactly where the prompt ends.
  // Measured caret positions are logicalLength + 2 in both base directions.
  var PROMPT = /^(?:[❯>»❱›] ?|[ ]{2})/;

  var MAX_LINE = 2000;

  function engine() {
    return typeof globalThis !== 'undefined' ? globalThis.__rtlBidi : null;
  }

  /* Reordering only, without bidi rule L4: the characters move but brackets
   * keep their shape. That is how Claude paints - it never applies L4 - so a
   * line containing brackets is only recognisable when reordered the same way.
   * bidi-js does apply L4 in getReorderedString, hence the manual permute. */
  function reorderOf(text) {
    var b = engine();
    if (!b) return null;
    var levels = b.getEmbeddingLevels(text, 'auto');
    var order = b.getReorderedIndices(text, levels, 0, text.length - 1);
    var out = new Array(order.length);
    for (var i = 0; i < order.length; i++) out[i] = text[order[i]];
    return { order: order, painted: out.join(''), levels: levels };
  }

  /* Every logical text whose bidi reordering is exactly `painted`.
   *
   * There is usually more than one. "שלום hello" and "hello שלום" paint
   * identically - one with an RTL base direction, one with an LTR one - but
   * their caret maps differ, so picking blindly makes the caret jump. */
  // A directional run swallows the neutrals inside it (bidi rule N1), so
  // "test 42" moves as one block. Splitting it on the space produces a
  // plausible-looking string that repaints correctly but maps the caret wrong.
  var LATIN_RUN = /[A-Za-z0-9](?:[A-Za-z0-9 ._\-\/:@#'"()]*[A-Za-z0-9])?/g;
  var HEBREW_RUN = /[֐-׿](?:[֐-׿ ._\-\/:@#'"()]*[֐-׿])?/g;

  function unreverseRuns(text, re) {
    var out = text.split('');
    var m;
    re.lastIndex = 0;
    while ((m = re.exec(text)) !== null) {
      var seg = m[0].split('').reverse();
      for (var k = 0; k < seg.length; k++) out[m.index + k] = seg[k];
    }
    return out.join('');
  }

  function candidates(painted) {
    if (!painted || painted.length > MAX_LINE) return [];
    var found = [], seen = Object.create(null);
    var flipped = painted.split('').reverse().join('');
    var guesses = [
      painted,
      flipped,
      unreverseRuns(flipped, LATIN_RUN),
      unreverseRuns(flipped, HEBREW_RUN),
      unreverseRuns(painted, LATIN_RUN),
      unreverseRuns(painted, HEBREW_RUN)
    ];
    for (var g = 0; g < guesses.length; g++) {
      var cand = guesses[g];
      for (var it = 0; it < 6; it++) {
        var r = reorderOf(cand);
        if (!r) return found;
        if (r.painted === painted) {
          if (!seen[cand]) {
            seen[cand] = 1;
            found.push({ text: cand, order: r.order, levels: r.levels });
          }
          break;
        }
        var next = new Array(painted.length);
        for (var i = 0; i < r.order.length; i++) next[r.order[i]] = painted[i];
        var joined = next.join('');
        if (joined === cand) break;
        cand = joined;
      }
    }
    return found;
  }

  function commonPrefix(a, b) {
    var n = Math.min(a.length, b.length), i = 0;
    while (i < n && a[i] === b[i]) i++;
    return i;
  }

  /* Typing grows the line and deleting shrinks it, so the text that continues
   * the row is one where the shorter of the two is a prefix of the longer.
   * Anything else is a different line - the input was cleared and something
   * new was typed - and a stray leading character in common says nothing.
   * Scoring those as zero keeps a new line from inheriting the old line's
   * direction. The trailing pad cell is not part of the comparison. */
  function continuationScore(text, prev) {
    var a = text.replace(/\s+$/, '');
    var b = prev.replace(/\s+$/, '');
    if (!b) return 0;
    var n = commonPrefix(a, b);
    return n === a.length || n === b.length ? n : 0;
  }

  // Last resolved logical text per input row. Typing grows the line one
  // character at a time, and the short prefix was unambiguous, so the candidate
  // that continues it is the right one. Deleting shrinks it, which the same
  // longest-common-prefix score handles.
  var memo = Object.create(null);

  function recover(painted, rowKey) {
    var found = candidates(painted);
    if (!found.length) return null;
    if (found.length > 1 && rowKey !== undefined) {
      var prev = memo[rowKey] || '';
      var best = 0, bestScore = -1;
      for (var i = 0; i < found.length; i++) {
        var score = continuationScore(found[i].text, prev);
        if (score > bestScore) { bestScore = score; best = i; }
      }
      found = [found[best]];
    }
    if (rowKey !== undefined) {
      memo[rowKey] = found[0].text;
      // Painting resolved this exact row with the typing history to break the
      // tie. A copy of the same row has no history of its own, so it reuses
      // the answer instead of guessing again.
      rememberPainted(painted, found[0].text);
    }
    return found[0];
  }

  var paintedMemo = Object.create(null);
  var paintedKeys = [];
  var PAINTED_MAX = 400;

  function rememberPainted(painted, logical) {
    if (paintedMemo[painted] === logical) return;
    if (paintedMemo[painted] === undefined) {
      paintedKeys.push(painted);
      if (paintedKeys.length > PAINTED_MAX) delete paintedMemo[paintedKeys.shift()];
    }
    paintedMemo[painted] = logical;
  }

  /* ---- diagnostics ------------------------------------------------------
   *
   * A ring of the decisions taken for rows that actually contain RTL, one
   * entry per distinct row content, readable from the editor's devtools
   * console as __rtlLog. Rows repaint many times per second, so only a change
   * in what the row says is recorded. */
  var LOG_MAX = 300;
  var log = [];
  var logSeen = Object.create(null);

  function record(entry) {
    var key = entry.kind + '|' + entry.row;
    var sig = entry.text + '|' + entry.caret;
    if (logSeen[key] === sig) return;
    logSeen[key] = sig;
    log.push(entry);
    if (log.length > LOG_MAX) log.shift();
  }

  function isRtlAt(levels, i) {
    try {
      var arr = levels && levels.levels;
      return !!arr && (arr[i] & 1) === 1;
    } catch (e) {
      return false;
    }
  }

  function baseIsRtl(levels) {
    try {
      var p = levels.paragraphs && levels.paragraphs[0];
      return !!p && (p.level & 1) === 1;
    } catch (e) {
      return false;
    }
  }

  /* Content span of a painted line: after the prompt, before trailing padding. */
  function spanOf(s) {
    var m = PROMPT.exec(s);
    var a = m ? m[0].length : 0;
    var e = s.length - 1;
    while (e >= a && WS.test(s[e])) e--;
    return { a: a, e: e };
  }

  var diag = null;

  function mapCaret(term, c) {
    try {
      var buf = term.buffer.active;
      var line = buf.getLine(buf.baseY + buf.cursorY);
      if (!line) return c;
      var s = line.translateToString(true);
      if (!RTL.test(s)) return c;
      diag = { kind: 'caret', row: buf.cursorY, text: s, caret: c, recovered: null };

      var sp = spanOf(s), a = sp.a, e = sp.e;
      if (e < a || c < a) return c;

      var rec = recover(s.slice(a, e + 1), buf.cursorY);
      if (!rec) return c;
      diag.recovered = rec.text;

      var n = rec.order.length;
      var d = c - a;

      if (d >= n) {
        // Past the last painted glyph, because the renderer trimmed a trailing
        // cell. Anchor against the last real character, then keep stepping in
        // the paragraph direction for anything beyond it.
        var last = n - 1;
        var lastVisual = rec.order.indexOf(last);
        if (lastVisual < 0) return c;
        var anchor = lastVisual + (isRtlAt(rec.levels, last) ? 0 : 1);
        var step = baseIsRtl(rec.levels) ? -1 : 1;
        var pos = anchor + step * (d - n);
        return a + (pos < 0 ? 0 : pos);
      }

      // A caret sits between characters and a line cursor draws on the left
      // edge of a cell, so the cell depends on the direction either side of the
      // gap. At a boundary those disagree; follow the character just typed,
      // which is the one before the caret. This is the usual strong-caret rule
      // and it is what makes "שלום h" put the caret after the h rather than at
      // the far left where the paragraph-level trailing cell sits.
      var j = d > 0 ? d - 1 : d;
      var v = rec.order.indexOf(j);
      if (v < 0) return c;
      var rtl = isRtlAt(rec.levels, j);
      if (d === 0) return a + v + (rtl ? 1 : 0);
      return a + v + (rtl ? 0 : 1);
    } catch (err) {
      return c;
    }
  }

  /* ---- optional: flush RTL rows to the right edge ---------------------- */

  // Box drawing, block elements and braille. Claude paints frames, separators
  // and progress glyphs with these; shifting a row that contains any of them
  // would tear the surrounding layout apart.
  // Box drawing and block elements only. The prompt glyph U+276F sits just
  // outside this range and must not be mistaken for a frame.
  var LAYOUT = /[─-▟⠀-⣿]/;

  var shiftCache = Object.create(null);
  var shiftCacheKeys = [];
  var SHIFT_CACHE_MAX = 400;
  var currentShift = 0;

  /* The base direction must come from the same resolution the caret uses.
   * Resolving it independently re-opens the ambiguity, and the row then
   * flips left the moment a Latin character makes a Hebrew-first line look
   * Latin-first - which is the alignment flicker seen while typing. */
  function computeShift(text, cols, rowKey) {
    if (!text || text.length > MAX_LINE) return 0;
    if (!RTL.test(text) || LAYOUT.test(text)) return 0;

    var end = text.length - 1;
    while (end >= 0 && WS.test(text[end])) end--;
    if (end < 0) return 0;

    var shift = cols - 1 - end;
    if (shift <= 0) return 0;

    var sp = spanOf(text);
    if (sp.e < sp.a) return 0;
    var rec = recover(text.slice(sp.a, sp.e + 1), rowKey);
    if (!rec || !baseIsRtl(rec.levels)) return 0;
    return shift;
  }

  function rowShift(line, cols, rowKey) {
    currentShift = 0;
    try {
      if (!line) return 0;
      var text = line.translateToString(true);
      var key = rowKey + '|' + cols + '|' + text;
      var hit = shiftCache[key];
      if (hit === undefined) {
        hit = computeShift(text, cols, rowKey);
        shiftCache[key] = hit;
        shiftCacheKeys.push(key);
        if (shiftCacheKeys.length > SHIFT_CACHE_MAX) {
          delete shiftCache[shiftCacheKeys.shift()];
        }
      }
      if (RTL.test(text)) {
        record({ kind: 'row', row: rowKey, text: text, caret: -1, cols: cols, shift: hit });
      }
      currentShift = hit;
      return hit;
    } catch (err) {
      return 0;
    }
  }

  /* Read the cell that should appear at column x. Shifting the source rather
   * than the destination keeps every column written exactly once, so no stale
   * cells are left behind. Columns before the shift read from the tail of the
   * line, which is blank precisely because the content was short enough to
   * shift in the first place. */
  function sourceColumn(x, cols) {
    if (!currentShift) return x;
    return x < currentShift ? cols - 1 : x - currentShift;
  }

  /* ---- bidi rule L4: mirrored glyphs ----------------------------------- */

  /* A mirrorable character resolved to an odd level is drawn as its mirror:
   * "(א)" reads as ")א(" once the run is laid out right to left. Claude
   * reorders the line but never applies L4, so every bracket in a Hebrew run
   * paints the wrong way round. The map is keyed by painted column and carries
   * the character expected in that cell, so a map computed for another line
   * cannot rewrite a cell it does not describe. */
  var MIRRORABLE = /[()\[\]{}<>«»‹›]/;

  var CODEPOINT_MASK = 0x1fffff;
  var COMBINED_MASK = 0x800000;

  function mirrorOf(ch) {
    var b = engine();
    if (!b || !b.getMirroredCharacter) return null;
    return b.getMirroredCharacter(ch);
  }

  function sameMirrors(a, b) {
    var k;
    for (k in a) if (!b[k] || b[k][1] !== a[k][1]) return false;
    for (k in b) if (!a[k]) return false;
    return true;
  }

  /* Only mirror when every logical text that repaints as this segment agrees.
   * Where they disagree the level is genuinely ambiguous from the paint alone,
   * and leaving the glyph as it is beats flipping it the wrong way. */
  function mirrorsOf(text, offset, out) {
    var found = candidates(text);
    if (!found.length) return;
    var first = null;
    for (var c = 0; c < found.length; c++) {
      var rec = found[c];
      var one = Object.create(null);
      for (var i = 0; i < rec.text.length; i++) {
        if (!(rec.levels.levels[i] & 1)) continue;
        var m = mirrorOf(rec.text[i]);
        if (!m) continue;
        var v = rec.order.indexOf(i);
        if (v < 0) continue;
        one[offset + v] = [rec.text.charCodeAt(i), m.charCodeAt(0)];
      }
      if (first === null) first = one;
      else if (!sameMirrors(first, one)) return;
    }
    for (var k in first) out[k] = first[k];
  }

  /* Frames and separators split a row into cells that were each reordered on
   * their own, so a table row is not the reordering of any single string. */
  function rowMirrors(text) {
    if (!text || text.length > MAX_LINE) return null;
    if (!RTL.test(text) || !MIRRORABLE.test(text)) return null;

    var out = Object.create(null);
    var any = false;
    // The prompt glyph is painted outside the reordered span and is itself
    // mirrorable, so it must not be resolved as part of the line.
    var start = spanOf(text).a;
    for (var i = start; i <= text.length; i++) {
      if (i < text.length && !LAYOUT.test(text[i])) continue;
      var a = start, e = i - 1;
      while (a <= e && WS.test(text[a])) a++;
      while (e >= a && WS.test(text[e])) e--;
      start = i + 1;
      if (e < a) continue;
      var seg = text.slice(a, e + 1);
      if (!RTL.test(seg) || !MIRRORABLE.test(seg)) continue;
      mirrorsOf(seg, a, out);
      any = true;
    }
    if (!any) return null;
    for (var k in out) return out;
    return null;
  }

  var mirrorCache = Object.create(null);
  var mirrorCacheKeys = [];
  var currentMirrors = null;

  function rowMirrorCached(text) {
    var hit = mirrorCache[text];
    if (hit === undefined) {
      hit = rowMirrors(text);
      mirrorCache[text] = hit;
      mirrorCacheKeys.push(text);
      if (mirrorCacheKeys.length > SHIFT_CACHE_MAX) {
        delete mirrorCache[mirrorCacheKeys.shift()];
      }
    }
    return hit;
  }

  function mirrorCell(x, cell) {
    try {
      var pair = currentMirrors && currentMirrors[x];
      if (!pair) return;
      var content = cell.content;
      if (content & COMBINED_MASK) return;
      if ((content & CODEPOINT_MASK) !== pair[0]) return;
      cell.content = (content & ~(CODEPOINT_MASK | COMBINED_MASK)) | pair[1];
    } catch (err) {
      /* leave the cell alone */
    }
  }

  /* Row prologue: everything the per-column loop below needs, resolved once. */
  function rowSetup(line, cols, rowKey) {
    currentShift = 0;
    currentMirrors = null;
    try {
      if (!line) return 0;
      var text = line.translateToString(true);
      if (target.__rtlMirrorGlyphs) currentMirrors = rowMirrorCached(text);
      return target.__rtlAlign ? rowShift(line, cols, rowKey) : 0;
    } catch (err) {
      return 0;
    }
  }

  /* ---- copy: hand back logical text, not painted order ------------------ */

  /* The buffer holds what Claude painted, which is visual order, and xterm
   * copies its cells verbatim. Paste that back and Claude reorders it a second
   * time, so the pasted run lands mirrored while everything typed around it
   * reads correctly. Recovering the logical text on the way out makes copy and
   * paste a round trip - and makes Hebrew copied into any other program come
   * out readable.
   *
   * A line whose recovery does not verify is handed back untouched, and no row
   * key is passed, so the per-row memo the caret depends on is not disturbed by
   * a selection. */
  function logicalLine(painted) {
    if (!painted || painted.length > MAX_LINE || !RTL.test(painted)) return painted;
    // The same span the caret works on: the prompt glyph and the wrap indent
    // were painted outside the reordered run, so folding them in would leave
    // nothing that verifies.
    var sp = spanOf(painted), a = sp.a, e = sp.e;
    if (e < a) return painted;
    var body = painted.slice(a, e + 1);

    // A line that reorders to itself needed no reordering, so what is on the
    // screen is already the logical text. Copying it verbatim is right, and
    // this is the case the tie-break below cannot see on its own.
    var self = reorderOf(body);
    if (!self || self.painted === body) return painted;

    var known = paintedMemo[body];
    var logical = known !== undefined ? known : null;
    if (logical === null) {
      var rec = recover(body);
      if (!rec) return painted;
      logical = rec.text;
    }
    return painted.slice(0, a) + logical + painted.slice(e + 1);
  }

  function caretShiftFor(term) {
    try {
      var buf = term.buffer.active;
      var line = buf.getLine(buf.baseY + buf.cursorY);
      if (!line) return 0;
      var text = line.translateToString(true);
      return computeShift(text, term.cols, buf.cursorY);
    } catch (err) {
      return 0;
    }
  }

  var target = typeof globalThis !== 'undefined' ? globalThis : this;
  target.__rtlCaret = function (term, c) {
    diag = null;
    var mapped = mapCaret(term, c);
    var shift = target.__rtlAlign ? caretShiftFor(term) : 0;
    if (diag) {
      diag.mapped = mapped;
      diag.shift = shift;
      record(diag);
      diag = null;
    }
    return shift ? mapped + shift : mapped;
  };
  target.__rtlCopy = function (lines) {
    try {
      if (!target.__rtlCopyLogical || !lines) return lines;
      for (var i = 0; i < lines.length; i++) lines[i] = logicalLine(lines[i]);
      return lines;
    } catch (err) {
      return lines;
    }
  };
  target.__rtlLog = log;
  target.__rtlRow = rowSetup;
  target.__rtlMirror = mirrorCell;
  target.__rtlSrc = sourceColumn;
  if (typeof module !== 'undefined' && module.exports) {
    module.exports = {
      mapCaret: mapCaret,
      recover: recover,
      spanOf: spanOf,
      candidates: candidates,
      computeShift: computeShift,
      sourceColumn: sourceColumn,
      setShift: function (n) { currentShift = n; },
      logicalLine: logicalLine,
      rowMirrors: rowMirrors,
      mirrorCell: mirrorCell,
      setMirrors: function (m) { currentMirrors = m; }
    };
  }
})();

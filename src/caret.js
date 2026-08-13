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
        var score = commonPrefix(found[i].text, prev);
        if (score > bestScore) { bestScore = score; best = i; }
      }
      found = [found[best]];
    }
    if (rowKey !== undefined) memo[rowKey] = found[0].text;
    return found[0];
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

  function mapCaret(term, c) {
    try {
      var buf = term.buffer.active;
      var line = buf.getLine(buf.baseY + buf.cursorY);
      if (!line) return c;
      var s = line.translateToString(true);
      if (!RTL.test(s)) return c;

      var sp = spanOf(s), a = sp.a, e = sp.e;
      if (e < a || c < a) return c;

      var rec = recover(s.slice(a, e + 1), buf.cursorY);
      if (!rec) return c;

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
    var mapped = mapCaret(term, c);
    if (!target.__rtlAlign) return mapped;
    var shift = caretShiftFor(term);
    return shift ? mapped + shift : mapped;
  };
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
      rowMirrors: rowMirrors,
      mirrorCell: mirrorCell,
      setMirrors: function (m) { currentMirrors = m; }
    };
  }
})();

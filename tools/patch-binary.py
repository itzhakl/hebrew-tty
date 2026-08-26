#!/usr/bin/env python3
"""Right-align RTL rows and put the caret on the glyph it edits, on any build.

Nothing here matches a minified name: every site is found by the shape of its
code and the names are read back out of the match. Verified on 2.1.241,
2.1.243 and 2.1.246, whose builds share no local identifiers at all.

What it does:

  * the bidi reorder function records the base direction, the visual->logical
    permutation and the levels on the array it returns,
  * the row painter flushes an RTL row to one column short of the right edge
    and memoises the row's geometry,
  * a caret map turns a logical column into a visual one, and the frame flush
    calls it.

The free column is the point. Claude puts the caret at the logical start of the
line, which in RTL is past the rightmost glyph; flush to the edge there is no
such column and the caret sticks on the first letter.

The binary is a Bun single-file executable addressed by byte offsets, so the
embedded JS must keep its exact length. Bytes are borrowed inside the chunk
that is being grown, which is why the caret map can live in a different chunk
from the row painter: 2.1.243 has only ~350 spare bytes where the painter is.

    python3 patch_v4.py <input-binary> <output-binary>
"""
import os
import re
import subprocess
import sys

# Sites are located by a cheap literal first, then matched by shape in a small
# window; running these patterns over 340MB directly takes minutes.
SITES = {
    "levels": (b'getEmbeddingLevels(', 400, 200,
               rb'\{levels:(\w+)\}=(\w+)\.getEmbeddingLevels\((\w+),"auto"\)'),
    "init":   (b'getEmbeddingLevels(', 200, 900,
               rb'let (\w+)=\[\.\.\.(\w+)\],(\w+)=Math\.max\(\.\.\.(\w+)\);'),
    "swap":   (b'getEmbeddingLevels(', 200, 900,
               rb'(\w+)\((\w+),(\w+),(\w+)-1\),(\w+)\((\w+),\3,\4-1\),\3=\4'),
    "ret":    (b'getEmbeddingLevels(', 200, 900,
               rb'(\w+)=(\w+)\}else \1\+\+\}return (\w+)\}'),
    "row":    (b'={char:" "', 300, 100, rb'let (\w+)=(\w+),(\w+)=\{char:" "'),
    "flush":  (b'.relativeX', 300, 200,
               rb'\{x:(\w+)\.x\+(\w+)\.relativeX,y:\1\.y\+\2\.relativeY\}'),
    # The input renderer's fork: with highlights it draws one <Text> per
    # highlighted run, without them one <Text> for the whole line.
    "hl":     (b'renderedRowStartOffsets', 200, 100,
               rb'(\w+)=(\w+)&&\2\.length>0\?(\w+)\((\w+),(\w+),'
               rb'(\w+)\.renderedRowStartOffsets\):\2;if\(\1&&\1\.length>0\)return '),
}
SIG = re.compile(rb'function (\w+)\((\w+),(\w+),(\w+),(\w+),(\w+),(\w+),(\w+)\)\{')
IMPORT_END = re.compile(rb'from"/\$bunfs/root/[A-Za-z0-9._-]{1,60}";')

# Rewrites that free bytes without changing meaning. `undefined` compares false
# against every numeric bound, so the void-0 guards in front of one are dead.
SHRINK = [
    (re.compile(rb'([,;{}()\[\]=>:!&|?])\(([A-Za-z_$][A-Za-z0-9_$]{0,20})\)=>'),
     lambda m: m.group(1) + m.group(2) + b"=>"),
    # Only a bare name: `a.b.length===0` would need the whole member chain.
    (re.compile(rb'(?<![.\w$])(\w+)\.length===0'), lambda m: b"!" + m.group(1) + b".length"),
    (re.compile(rb'(?<![.\w$])(\w+)!==void 0&&\1(<=|>=|<|>)'), lambda m: m.group(1) + m.group(2)),
    # `typeof` yields a string, so === against a string literal is ==.
    (re.compile(rb'(typeof [\w.$]{1,24})===("[a-z]{3,9}")'),
     lambda m: m.group(1) + b"==" + m.group(2)),
    (re.compile(rb'(typeof [\w.$]{1,24})!==("[a-z]{3,9}")'),
     lambda m: m.group(1) + b"!=" + m.group(2)),
]

D, P, M, W, S, I, RQ = b"$rd_", b"$rp_", b"$rm_", b"$rw_", b"$rs_", b"$ri_", b"$rq_"
RN, RF, RT = b"$rn_", b"$rf_", b"$rt_"


def find(buf, key):
    lit, back, fwd, pat = SITES[key]
    hits, i = [], buf.find(lit)
    while i >= 0:
        for m in re.finditer(pat, buf[i - back:i + fwd]):
            hits.append((i - back + m.start(), m))
        i = buf.find(lit, i + 1)
    if len(hits) != 1:
        sys.exit(f"{key}: expected one match, found {len(hits)}")
    off, m = hits[0]
    return off, off + (m.end() - m.start()), m


HEADER = re.compile(rb'\x00// @bun @bytecode\n')
IMPORTS = re.compile(rb'import\{([^}]{1,40000})\}from"/\$bunfs/root/([A-Za-z0-9._/-]{1,80})"')
EXPORTS = re.compile(rb'export\{([^}]{1,40000})\};')


def chunks(buf):
    """Source spans, one per embedded module, named by whoever imports them.

    A module opens with its `@bun @bytecode` banner and its source runs to the
    NUL that ends it. Where the bytecode sits relative to that source is a
    build's business and it has already moved once: until 2.1.245 each
    module's constant pool followed its source, and 2.1.246 gathered every
    pool into one region ahead of every source. Reading the layout off the
    pools tracked neither - it named a span with the pool of the span before.

    The name comes from the other side instead. A module exports aliases
    carrying a suffix of its own, so the import statement naming those aliases
    names the module too, and having found one the module is statically
    imported by construction. A span nobody imports stays unnamed and is never
    offered the payload.
    """
    starts = [m.start() + 1 for m in HEADER.finditer(buf)]
    if not starts:
        sys.exit("no module layout found")
    where = {}
    for m in IMPORTS.finditer(buf):
        for part in m.group(1).split(b","):
            where.setdefault(part.split(b" as ")[0], m.group(2))
    out = []
    for lo in starts:
        hi = buf.find(b"\x00", lo)
        last = None
        for m in EXPORTS.finditer(buf, lo, hi):
            last = m
        name = None
        for part in (last.group(1).split(b",") if last else []):
            name = where.get(part.split(b" as ")[-1])
            if name:
                break
        out.append((lo, hi, name.decode() if name else None))
    return out


def host_of(spans, off):
    for lo, hi, name in spans:
        if lo <= off < hi:
            return lo, hi, name
    sys.exit(f"offset {off} is in no chunk")


def derive(buf, check=False):
    if check:
        for ident in (D, P, M, W, S, I, RQ, RN, RF, RT):
            if ident in buf:
                sys.exit(f"identifier {ident.decode()} is already taken")
    d = {k: find(buf, k) for k in SITES}

    row_start = d["row"][0]
    sig = None
    for m in SIG.finditer(buf, row_start - 4000, row_start):
        sig = m
    if sig is None:
        sys.exit("row painter signature not found")
    d["sig"] = (sig.start(), sig.end(), sig)

    reorder = re.search(rb'function (\w+)\(\w+\)\{', buf[d["levels"][0] - 500:d["levels"][0]])
    if reorder is None:
        sys.exit("reorder function name not found")
    d["reorder"] = reorder.group(1)
    seg = re.search(rb'(\w+)=' + re.escape(d["reorder"]) + rb'\(', buf[sig.start():row_start])
    if seg is None:
        sys.exit("segments variable not found")
    d["segments"] = seg.group(1)

    # The call itself, so the reorder can be handed a row one cell at a time.
    at = sig.start() + seg.end() - 1
    depth, i = 0, at
    while i < row_start:
        if buf[i:i + 1] == b"(":
            depth += 1
        elif buf[i:i + 1] == b")":
            depth -= 1
            if depth == 0:
                break
        i += 1
    if depth != 0:
        sys.exit("reorder call not balanced")
    d["call"] = (sig.start() + seg.end() - len(d["reorder"]) - 1, i + 1, buf[at + 1:i])
    return d


def caret_map():
    """Logical caret column -> visual, anchored on the character before it."""
    return (
        b"globalThis." + RQ + b"=function(y,x){let m=globalThis." + M + b",R=m&&m[y];"
        b"if(!R)return x;let S=R.S,P=R.P,L=R.L,q=P.length,V=[],Q=[],c=R.x;"
        b"for(let v=0;v<q;v++)V[v]=c,c+=S[v].width||1,Q[P[v]]=v;"
        b"let d=0,w=R.r;while(d<q){let g=S[Q[d]].width||1;if(w+g>x)break;w+=g,d++}"
        b"let j=d?d-1:0,v=Q[j],b=S[v].width||1,o=L[v]&1;"
        b"return d?V[v]+(o?0:b):V[v]+(o?b:0)};"
    )


def base_dir():
    """Base direction by the majority of strong characters, not the first one.

    "auto" is bidi rule P2: the paragraph takes the direction of its first
    strong character. A Hebrew line that opens with a path, a flag or a
    version number therefore lays out left to right - the sentence reads
    backwards and its full stop lands on the wrong side. Counting decides it
    instead, and a line with no Hebrew in it is left on "auto" exactly as
    before.
    """
    return (
        b"globalThis." + RF + b"=function(t){"
        b"var r=t.match(/[\\u0590-\\u08ff]/g),l=t.match(/[A-Za-z]/g);"
        b'return r&&r.length>=(l?l.length:0)?"rtl":"auto"};'
    )


def per_cell():
    """A table row is not a paragraph. Every cell in it is.

    Claude reorders a painted row in one go, so a row of table cells is laid
    out as one sentence: the column rules travel with the text, the cells land
    under the wrong headings, and a row whose cells are mostly Latin keeps an
    order the row above it does not. This is the rule the editor patch already
    applies to a multiplexer's panes, one level down - the row is cut at every
    vertical rule and each piece is reordered against itself, while the rules
    themselves stay where they were. That is what holds the borders still.

    Such a row is never flushed right, so it needs no permutation, and it
    carries no levels either: bidi rule L4 reads those, so a bracket inside a
    table cell keeps the glyph it was typed as. Paying 90 bytes of payload for
    that would cost the whole helper its place in the one chunk with room.
    """
    return (
        b"globalThis." + RT + b"=function(f,s){"
        b'var V="\\u2502",n=s.length,i,h=0;'
        b"for(i=0;i<n;i++)if(s[i].value===V){h=1;break}"
        b"if(!h)return f(s);"
        b"var o=[],a=0;"
        b"for(i=0;i<=n;i++){if(i<n&&s[i].value!==V)continue;"
        b"if(i>a)o.push.apply(o,f(s.slice(a,i)));"
        b"if(i<n)o.push(s[i]);a=i+1}"
        b"return o};"
    )


def line_pass():
    """One walk of a reordered line: bidi rule L4, and whether it is a layout row.

    L4 is the rule Claude Code skips, so a bracket inside an RTL run keeps its
    unmirrored glyph. The layout flag is the same test src/caret.js makes at
    LAYOUT: a row carrying box drawing is part of a frame, and flushing it to
    the right edge tears it away from the borders that hold still around it.

    The reordered segment array is cached per source line, so the pass is
    marked on the array itself. Running the mirror twice would swap every
    bracket back, which looks exactly like not running it at all.
    """
    return (
        b"globalThis." + RN + b"=function(" + S + b"){"
        b"if(" + S + b".RM||!" + S + b".RL)return;" + S + b".RM=1;"
        b'var B="()[]{}<>",'
        b"L=/[\\u2500-\\u259f\\u2800-\\u28ff]/,V=0;"
        b"for(var " + I + b"=0;" + I + b"<" + S + b".length;" + I + b"++){"
        b"var c=" + S + b"[" + I + b"].value;"
        b"if(L.test(c))V=1;"
        b"if(!(" + S + b".RL[" + I + b"]&1))continue;"
        b"var j=B.indexOf(c);"
        b"if(j>=0)" + S + b"[" + I + b"][\"value\"]=B[j^1]}"
        + S + b".RV=V};"
    )



def one_run(buf):
    """Keep a line that holds RTL out of the highlighted renderer.

    Claude draws the prompt input as a single <Text> - one write op, one bidi
    reorder, one row to align - until something wants part of it coloured. A
    highlight splits the row into one <Text> per run, and Ink then emits a
    write op per run. Reordering and right-alignment are per op, so two RTL
    runs both flush to the right edge and the second paints over the first:
    the row goes blank while it is being dictated, and the caret follows a
    fragment rather than the line.

    Dictation is the case that always hits it - the interim transcript is
    painted as a dim highlight for as long as the microphone is open - but a
    keyword or a mention splits the row the same way.

    Nothing here reorders anything. It only says that a line with RTL in it
    takes the path that already works, and loses its colouring while it does.
    """
    lo, hi, m = find(buf, "hl")
    tail = b")return "
    return (lo, hi,
            m.group(0)[:-len(tail)] + b"&&!/[\\u0590-\\u08ff]/.test("
            + m.group(5) + b")" + tail)


def edits(d):
    lv, it, sw, rt, rw, fl = (d[k][2] for k in ("levels", "init", "swap", "ret", "row", "flush"))
    sig = d["sig"][2]
    levels, recv, text = lv.groups()
    segs, src, mx, lvls = it.groups()
    rev, _arr, c, u, _rev2, lvarr = sw.groups()
    start, logical, cell = rw.groups()
    k, t = fl.groups()
    row_idx, width = sig.group(5), sig.group(6)
    seg = d["segments"]
    ret = rt.group(3)
    y = k + b".y+" + t + b".relativeY"

    return [
        (d["levels"][0], d["levels"][1],
         b"{levels:" + levels + b",paragraphs:" + D + b"}=" + recv
         + b".getEmbeddingLevels(" + text + b",globalThis." + RF + b"?.(" + text
         + b')??"auto")'),
        (d["init"][0], d["init"][1],
         b"let " + segs + b"=[..." + src + b"]," + P + b"=" + segs + b".map((_," + I
         + b")=>" + I + b")," + mx + b"=Math.max(..." + lvls + b");"),
        (d["swap"][0], d["swap"][1],
         sw.group(0)[:-len(c + b"=" + u)] + rev + b"(" + P + b"," + c + b"," + u
         + b"-1)," + c + b"=" + u),
        (d["ret"][0], d["ret"][1],
         rt.group(0)[:-len(b"return " + ret + b"}")] + b"return " + ret + b".RA=" + D
         + b"[0]?.level===1," + ret + b".RP=" + P + b"," + ret + b".RL=" + lvarr + b","
         + ret + b"}"),
        (d["row"][0], d["row"][1],
         b"globalThis." + RN + b"?.(" + seg + b");"
         b"let " + start + b"=" + logical + b"," + M + b"=globalThis." + M + b"??={};"
         b"if(" + seg + b".RA){let " + W + b"=0;for(let " + S + b" of " + seg + b")"
         + W + b"+=" + S + b".width;if(!" + seg + b".RV&&" + width + b"-" + W
         + b"-1>" + start + b")" + start + b"=" + width + b"-" + W + b"-1;" + M + b"[" + row_idx + b"]={x:"
         + start + b",r:" + logical + b",P:" + seg + b".RP,S:" + seg + b",L:" + seg
         + b".RL}}else " + M + b"[" + row_idx + b"]=0;let " + cell + b"={char:\" \""),
        # Falls back to the logical column if the caret map's chunk is not loaded.
        (d["call"][0], d["call"][1],
         b"(globalThis." + RT + b"??((f,x)=>f(x)))(" + d["reorder"] + b","
         + d["call"][2] + b")"),
        (d["flush"][0], d["flush"][1],
         b"{x:(globalThis." + RQ + b"??((a,b)=>b))(" + y + b"," + k + b".x+" + t
         + b".relativeX),y:" + y + b"}"),
    ]


def free_bytes(buf, lo, hi, wanted, skip):
    """Shorten equivalent constructs in [lo, hi) until `wanted` bytes are free."""
    picks, freed = [], 0
    for pattern, rewrite in SHRINK:
        for m in pattern.finditer(buf, lo, hi):
            if any(a <= m.start() < b for a, b in skip):
                continue
            gain = (m.end() - m.start()) - len(rewrite(m))
            if gain <= 0:
                continue
            picks.append((m.start(), m.end(), rewrite(m)))
            freed += gain
            if freed >= wanted:
                break
        if freed >= wanted:
            break
    if freed < wanted:
        sys.exit(f"only {freed} bytes free in this chunk, need {wanted}")
    picks.sort()
    pieces, prev = [], 0
    for a, b, new in picks:
        if a < prev:
            continue
        pieces.append(buf[prev:a])
        pieces.append(new)
        prev = b
    pieces.append(buf[prev:])
    return b"".join(pieces), freed


# How far after an edit we may look for the bytes to pay for it. Everything in
# between shifts, so the search stops at the end of the module: since 2.1.246
# the bytecode addresses source by offset and a byte that leaves its module is
# a boot that dies on a parameter list that is not there. Whether a shift
# inside one module is survivable is settled by booting the result, which is
# what the check at the end now does.


# How far past an edit we may look for the bytes that pay for it. Everything in
# between shifts, and that run is the whole risk - too narrow and there is
# nothing left to shrink, too wide and the build no longer starts. Which width
# a given build wants is not worth reasoning about: these are tried in order
# and the first whose result comes up on a pty is the one that ships.
WINDOWS = (200_000, 400_000, 1_000_000, 2_000_000)


def pay_in_place(buf, plan, window):
    """Rewrite each edit's own neighbourhood so the module holds still.

    Since 2.1.246 the bytecode addresses source by offset, and a function whose
    text has moved is parsed from the wrong place - the boot dies on a
    parameter list that is not there. A byte may never leave the module it was
    compiled with, and even inside one the region that shifts has to stay
    small: paying for every edit out of one pooled region moved a quarter of a
    megabyte and 2.1.246 refused to start. So each edit is paid for on its own,
    from the constructs that follow it, nearest first. Paying from in front of
    an edit as well was tried and 2.1.246 would not start: what shifts has to
    be the run between an edit and the code that pays for it, and nothing
    before it. That run does cross into the modules that follow, which the
    boot tolerates; the pooled version, which shifted a quarter of a megabyte
    around all seven edits at once, it did not.

    Whether a given shift is survivable is not something to reason about. The
    check at the end starts the build on a pty and watches for the interface.
    """
    plan = sorted(plan)
    spans = [(lo, hi) for lo, hi, _ in plan]
    mod_lo, mod_hi, name = host_of(chunks(buf), plan[0][0])
    used, reaches, total, paid = [], [], 0, 0
    # A region reaches as far as its payment, which is further than the next
    # edit: the regions nest. Each one is therefore cut from a buffer that
    # already carries the edits above it, or applying them lowest-last would
    # put back the code they replaced. Every region holds its length, so an
    # offset found here stays valid after one is applied.
    work = buf

    # Last edit first: each one claims the slack directly after itself before
    # an earlier one can reach past it and take it.
    for lo, hi, text in reversed(plan):
        need = len(text) - (hi - lo)
        total += need
        picks = []
        for pattern, rewrite in SHRINK:
            for m in pattern.finditer(work, hi, hi + window):
                if any(a < m.end() and m.start() < b for a, b in spans + used):
                    continue
                shrunk = rewrite(m)
                gain = (m.end() - m.start()) - len(shrunk)
                if gain > 0:
                    picks.append((m.start(), m.end(), shrunk, gain))
        picks.sort()

        chosen, freed = [], 0
        for pick in picks:
            if freed >= need:
                break
            if any(a < pick[1] and pick[0] < b for a, b, *_ in chosen):
                continue
            chosen.append(pick)
            freed += pick[3]
        if freed < need:
            sys.exit(f"{name}: an edit needs {need}, only {freed} left to shrink")
        paid += freed

        parts = sorted([(lo, hi, text)] + [(a, b, t) for a, b, t, _ in chosen])
        start, end = parts[0][0], parts[-1][1]
        body = bytearray(work[start:end])
        for a, b, t in reversed(parts):
            body[a - start:b - start] = t
        body = bytes(body) + b" " * (end - start - len(body))
        work = work[:start] + body + work[end:]
        used.append((start, end))
        reaches.append(end - start)

    print(f"  {name}: needs {total}, paid {paid}, widest shift {max(reaches)} bytes")
    return work


def boots(path):
    """Start the build on a pty and watch for the TUI rather than a version.

    `--version` prints from a handful of modules and says nothing about the
    rest of the graph: the build that died on a stale bytecode offset reported
    its version perfectly and then refused to start. Only a real terminal
    exercises the code this patch edits.
    """
    import fcntl
    import pty
    import select
    import struct
    import termios
    import time

    pid, fd = pty.fork()
    if pid == 0:
        os.execv(path, ["claude"])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
    seen, deadline = b"", time.time() + 60
    while time.time() < deadline:
        ready, _, _ = select.select([fd], [], [], 0.5)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 65536)
        except OSError:
            break
        if not chunk:
            break
        seen += chunk
        if b"could not start" in seen or len(seen) > 4000:
            break
    try:
        os.kill(pid, 15)
        os.waitpid(pid, 0)
    except OSError:
        pass
    os.close(fd)
    if b"could not start" in seen:
        print("  " + seen.decode(errors="replace").split("could not start")[1][:120].strip())
        return False
    return len(seen) > 200


def write_out(src, dst, buf):
    """Clone the stock binary, then write back only what the patch moved.

    The patch holds the file's length and touches a few hundred kilobytes of
    it. On a filesystem with reflinks the clone costs no space at all and the
    patched build costs what it actually changed, rather than a second copy of
    a 370MB executable per Claude version. Without reflinks the clone is a
    real copy and this is the plain write it always was.
    """
    BLOCK = 1 << 16
    clone = subprocess.run(["cp", "--reflink=auto", "--preserve=mode", src, dst])
    if clone.returncode != 0:
        open(dst, "wb").write(buf)
        subprocess.run(["chmod", "+x", dst], check=True)
        return
    written = 0
    with open(src, "rb") as old, open(dst, "r+b") as new:
        at = 0
        while True:
            block = old.read(BLOCK)
            if not block:
                break
            if block != buf[at:at + len(block)]:
                new.seek(at)
                new.write(buf[at:at + len(block)])
                written += len(block)
            at += len(block)
    subprocess.run(["chmod", "+x", dst], check=True)
    print(f"cloned, rewrote {written // 1024} KiB")


def build(buf, window):
    """Everything the patch does, at one payment width."""

    d = derive(buf, check=True)
    spans = chunks(buf)
    row_chunk = host_of(spans, d["row"][0])
    flush_chunk = host_of(spans, d["flush"][0])
    print(f"reorder {d['reorder'].decode()}, segments {d['segments'].decode()}, "
          f"painter in {row_chunk[2]}, flush in {flush_chunk[2]}")

    # 1. The payload goes wherever there is room, reached through globalThis.
    # It is placed one helper at a time and spread over as many chunks as it
    # takes: no single chunk has room for all of it, and a helper is
    # self-contained, so where each one lands does not matter.
    body = caret_map() + line_pass() + base_dir() + per_cell()
    spans = chunks(buf)
    # A chunk nothing imports statically is never instantiated, and a payload
    # left in one is dead without saying so. `import()` does not count: the
    # chunk that carried only a dynamic import took the whole payload with it
    # and every helper silently fell back to doing nothing.
    others = [c for c in spans
              if not (c[0] <= d["row"][0] < c[1]) and c[1] - c[0] >= 100_000
              and c[2] is not None]
    others.sort(key=lambda c: c[1] - c[0], reverse=True)

    placed = None
    for lo, hi, name in others:
        if IMPORT_END.search(buf, lo, min(hi, lo + 5_000_000)) is None:
            continue
        try:
            free_bytes(buf, lo, hi, len(body), [])
        except SystemExit:
            continue
        placed = (lo, hi, name)
        break
    if placed is None:
        sys.exit(f"no imported chunk has room for {len(body)} bytes of payload")

    lo, hi, name = placed
    buf, freed = free_bytes(buf, lo, hi, len(body), [])
    print(f"  payload -> {name}: needs {len(body)}, freed {freed}")
    at = IMPORT_END.search(buf, lo, min(hi, lo + 5_000_000)).end()
    buf = buf[:at] + body + b" " * (freed - len(body)) + buf[at:]

    # 2. The painter's edits, each paid for in the bytes right after it.
    d = derive(buf)
    plan = edits(d)
    buf = pay_in_place(buf, plan, window)

    # 3. The input renderer lives in a module of its own and pays for itself
    #    there, by the same rule.
    tail = one_run(buf)
    buf = pay_in_place(buf, [tail], window)

    # An edit that a later region put back leaves a build that still starts:
    # the payload is injected and every call site is written to do nothing
    # when its helper is missing. Nothing downstream would notice, so the
    # edits are read back here.
    for _lo, _hi, text in plan + [tail]:
        if buf.count(text) != 1:
            sys.exit(f"an edit is not in the result: {text[:60]!r}")

    return buf


def main():
    src, dst = sys.argv[1], sys.argv[2]
    original = open(src, "rb").read()

    for window in WINDOWS:
        print(f"payment width {window // 1000}k")
        try:
            buf = build(original, window)
        except SystemExit as why:
            print(f"  {why}")
            continue
        if len(buf) != len(original):
            sys.exit(f"length changed by {len(buf) - len(original)}")
        write_out(src, dst, buf)
        if boots(os.path.abspath(dst)):
            print(f"length held at {len(original)}, boots")
            return
    os.path.exists(dst) and os.remove(dst)
    sys.exit("no payment width produced a build that starts")


def selftest(path):
    """Check the anchors against code recorded from real binaries.

    The minifier renames everything on each build, so what has to keep working
    is the *shape* of each site and the names read back out of it. The fixture
    holds the windows those sites were found in, one set per version.
    """
    import json
    fixture = json.load(open(path))
    failures = 0
    for version, sites in sorted(fixture.items()):
        names, row_window = {}, None
        for key, (_lit, _back, _fwd, pat) in SITES.items():
            hits = []
            for window in sites[key]:
                found = list(re.finditer(pat, window.encode()))
                if found and key == "row":
                    row_window = window.encode()
                hits += found
            if len(hits) != 1:
                print(f"  {version} {key}: expected one match, found {len(hits)}")
                failures += 1
                continue
            names[key] = [g.decode() for g in hits[0].groups()]
        window = row_window
        sig = None
        for m in SIG.finditer(window or b""):
            sig = m
        if sig is None:
            print(f"  {version}: row painter signature not found")
            failures += 1
            continue
        width, row_idx = sig.group(6).decode(), sig.group(5).decode()
        print(f"  {version}: painter {sig.group(1).decode()}"
              f"(row={row_idx}, width={width}), reorder vars {names['ret']}")
    if failures:
        sys.exit(f"{failures} anchor checks failed")
    print(f"  all anchors resolve on {len(fixture)} recorded versions")


if len(sys.argv) == 3 and sys.argv[1] == "--selftest":
    selftest(sys.argv[2])
else:
    main()

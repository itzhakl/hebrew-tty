#!/usr/bin/env python3
"""Right-align RTL rows and put the caret on the glyph it edits, on any build.

Nothing here matches a minified name: every site is found by the shape of its
code and the names are read back out of the match. Verified on 2.1.241 and
2.1.243, whose builds share no local identifiers at all.

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
BLOB = re.compile(rb'file:///\$bunfs/root/([A-Za-z0-9._-]{1,60})')
IMPORT_END = re.compile(rb'from"/\$bunfs/root/[A-Za-z0-9._-]{1,60}";')

# Rewrites that free bytes without changing meaning. `undefined` compares false
# against every numeric bound, so the void-0 guards in front of one are dead.
SHRINK = [
    (re.compile(rb'([,;{}()\[\]=>:!&|?])\(([A-Za-z_$][A-Za-z0-9_$]{0,20})\)=>'),
     lambda m: m.group(1) + m.group(2) + b"=>"),
    # Only a bare name: `a.b.length===0` would need the whole member chain.
    (re.compile(rb'(?<![.\w$])(\w+)\.length===0'), lambda m: b"!" + m.group(1) + b".length"),
    (re.compile(rb'(?<![.\w$])(\w+)!==void 0&&\1(<=|>=|<|>)'), lambda m: m.group(1) + m.group(2)),
]

D, P, M, W, S, I, RQ = b"$rd_", b"$rp_", b"$rm_", b"$rw_", b"$rs_", b"$ri_", b"$rq_"
RN, RF = b"$rn_", b"$rf_"


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


NAME = re.compile(rb'/\$bunfs/root/[A-Za-z0-9._-]{1,60}')


def chunks(buf):
    """Source spans, one per embedded module, ending at its metadata blob.

    Builds before 2.1.243 carry no per-module blobs; there the whole embedded
    bundle between the name table's ends is one span.
    """
    out, prev = [], 0
    for m in BLOB.finditer(buf):
        out.append((prev, m.start(), m.group(1).decode()))
        prev = m.start()
    # One stray blob is not a layout; 2.1.243 carries well over a thousand.
    if len(out) >= 16:
        return out
    names = [m for m in NAME.finditer(buf)]
    if not names:
        sys.exit("no module layout found")
    return [(names[0].end(), names[-1].start(), "bundle")]


def host_of(spans, off):
    for lo, hi, name in spans:
        if lo <= off < hi:
            return lo, hi, name
    sys.exit(f"offset {off} is in no chunk")


def derive(buf, check=False):
    if check:
        for ident in (D, P, M, W, S, I, RQ, RN, RF):
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
        b'var B="()[]{}<>\\u00ab\\u00bb\\u2039\\u203a",'
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


def apply_in_chunk(buf, plan, extra_note=""):
    """Grow the sites in one chunk, paying for it inside that same chunk."""
    grown = sum(len(new) - (hi - lo) for lo, hi, new in plan)
    spans = [(lo, hi) for lo, hi, _ in plan if hi > lo]
    lo, hi, name = host_of(chunks(buf), plan[0][0])
    buf, freed = free_bytes(buf, lo, hi, grown, spans)
    print(f"  {name}: needs {grown}, freed {freed}{extra_note}")
    return buf, grown, freed, name


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


def main():
    src, dst = sys.argv[1], sys.argv[2]
    buf = open(src, "rb").read()
    original = len(buf)

    d = derive(buf, check=True)
    spans = chunks(buf)
    row_chunk = host_of(spans, d["row"][0])
    flush_chunk = host_of(spans, d["flush"][0])
    print(f"reorder {d['reorder'].decode()}, segments {d['segments'].decode()}, "
          f"painter in {row_chunk[2]}, flush in {flush_chunk[2]}")

    # 1. The caret map goes wherever there is room, reached through globalThis.
    # Its own chunk is the last resort: on 2.1.243 the painter's chunk has only
    # a few hundred spare bytes and the map alone is larger than that.
    body = caret_map() + line_pass() + base_dir()
    others = [s for s in spans if not (s[0] <= d["row"][0] < s[1]) and s[1] - s[0] >= 100_000]
    others.sort(key=lambda s: s[1] - s[0], reverse=True)

    placed = None
    for lo, hi, name in others:
        head = IMPORT_END.search(buf, lo, min(hi, lo + 5_000_000))
        if head is None:
            continue
        try:
            free_bytes(buf, lo, hi, len(body), [])
        except SystemExit:
            continue
        placed = (lo, hi, name, "import")
        break
    if placed is None:
        placed = (row_chunk[0], row_chunk[1], row_chunk[2], "painter")

    lo, hi, name, where = placed
    buf, freed = free_bytes(buf, lo, hi, len(body), [])
    print(f"  caret map -> {name}: needs {len(body)}, freed {freed}")
    if where == "import":
        at = IMPORT_END.search(buf, lo, min(hi, lo + 5_000_000)).end()
    else:
        d2 = derive(buf)
        at = d2["sig"][0]
    buf = buf[:at] + body + b" " * (freed - len(body)) + buf[at:]

    # 2. Everything else is paid for inside the painter's own chunk.
    d = derive(buf)
    plan = edits(d)
    buf, grown, freed2, cname = apply_in_chunk(buf, plan)
    d = derive(buf)
    plan = edits(d)
    for a, b, new in sorted(plan, reverse=True):
        buf = buf[:a] + new + buf[b:]
    sig = SIG.search(buf, d["row"][0] - 4000)
    buf = buf[:sig.end()] + b" " * (freed2 - grown) + buf[sig.end():]

    # 3. The input renderer lives in its own chunk and pays for itself there.
    buf, grown3, freed3, _ = apply_in_chunk(buf, [one_run(buf)])
    lo3, hi3, new3 = one_run(buf)
    buf = buf[:lo3] + new3 + b" " * (freed3 - grown3) + buf[hi3:]

    if len(buf) != original:
        sys.exit(f"length changed by {len(buf) - original}")

    write_out(src, dst, buf)
    print(f"length held at {original}")

    out = subprocess.run([dst, "--version"], capture_output=True, timeout=180)
    version = out.stdout.decode(errors="replace").strip()
    print("version:", version or "(empty - module graph broken)")
    if "Claude Code" not in version:
        sys.exit(1)


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

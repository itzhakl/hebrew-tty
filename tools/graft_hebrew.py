#!/usr/bin/env python3
"""Graft the Hebrew glyphs of a donor font onto a monospace base font.

The donor outlines are scaled to the base's units-per-em, resized so their
x-height matches the base, and re-centred inside the base's fixed advance, so
the result stays monospace and the terminal cell grid is unchanged.
"""
import sys
from fontTools.ttLib import TTFont
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.pens.transformPen import TransformPen
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.boundsPen import BoundsPen
from fontTools.misc.transform import Transform

HEBREW = range(0x0590, 0x0600)


def graft(base_path, donor_path, out_path, family, subfamily, height_ratio=1.0):
    base, donor = TTFont(base_path), TTFont(donor_path)
    b_upem, d_upem = base["head"].unitsPerEm, donor["head"].unitsPerEm
    b_cmap, d_cmap = base.getBestCmap(), donor.getBestCmap()
    b_glyf, b_hmtx = base["glyf"], base["hmtx"]
    d_gs = donor.getGlyphSet()
    adv = b_hmtx[b_cmap[0x61]][0]

    # match the donor's alef height to the base's alef height, not just the upem
    def height(font_gs, name):
        bp = BoundsPen(font_gs)
        font_gs[name].draw(bp)
        return (bp.bounds[3] - bp.bounds[1]) if bp.bounds else 0

    b_alef = height(base.getGlyphSet(), b_cmap[0x5D0]) if 0x5D0 in b_cmap else 0
    d_alef = height(d_gs, d_cmap[0x5D0]) if 0x5D0 in d_cmap else 0
    scale = (b_alef / d_alef) * height_ratio if b_alef and d_alef else b_upem / d_upem

    n = 0
    for code in HEBREW:
        if code not in d_cmap or code not in b_cmap:
            continue
        rec = DecomposingRecordingPen(d_gs)
        d_gs[d_cmap[code]].draw(rec)
        if not rec.value:
            continue
        pen = TTGlyphPen(None)
        rec.replay(TransformPen(pen, Transform().scale(scale)))
        g = pen.glyph()
        name = b_cmap[code]
        b_glyf[name] = g
        g.recalcBounds(b_glyf)
        width = g.xMax - g.xMin if g.numberOfContours else 0
        b_hmtx[name] = (adv, int((adv - width) / 2 - g.xMin) if width else adv)
        n += 1

    nm = base["name"]
    full = f"{family} {subfamily}".strip()
    for nid, val in ((1, family), (2, subfamily), (3, full + ";graft"), (4, full),
                     (6, full.replace(" ", "")), (16, family), (17, subfamily)):
        nm.setName(val, nid, 3, 1, 0x409)
    base.save(out_path)
    print(f"{out_path}: grafted {n} glyphs, scale {scale:.3f}, advance {adv}")


if __name__ == "__main__":
    a = sys.argv[1:]
    graft(a[0], a[1], a[2], a[3], a[4], float(a[5]) if len(a) > 5 else 1.0)

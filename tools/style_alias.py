#!/usr/bin/env python3
"""Re-label a grafted font as its italic face.

A terminal renders an italic run from a real italic face. If the grafted family
has no italic, Hebrew in that run falls back to a proportional font and leaves
the cell grid, which is the one thing the graft exists to prevent. Hebrew has no
italic form, so the upright outlines are re-used as-is and only the style bits
and names change.
"""
import sys
from fontTools.ttLib import TTFont

ITALIC, BOLD, REGULAR = 0x01, 0x20, 0x40


def alias(src_path, out_path, family, subfamily, bold=False):
    font = TTFont(src_path)
    font["head"].macStyle = (font["head"].macStyle | 0x02) | (0x01 if bold else 0)
    sel = font["OS/2"].fsSelection & ~REGULAR
    font["OS/2"].fsSelection = sel | ITALIC | (BOLD if bold else 0)

    nm = font["name"]
    full = f"{family} {subfamily}".strip()
    for nid, val in ((1, family), (2, subfamily), (3, full + ";graft"), (4, full),
                     (6, full.replace(" ", "")), (16, family), (17, subfamily)):
        nm.setName(val, nid, 3, 1, 0x409)
    font.save(out_path)
    print(f"{out_path}: {full}, macStyle {font['head'].macStyle}")


if __name__ == "__main__":
    a = sys.argv[1:]
    alias(a[0], a[1], a[2], a[3], len(a) > 4 and a[4] == "bold")

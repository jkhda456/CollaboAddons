#!/usr/bin/env python3
"""type.xml for the imagemagick add-on: the fonts ImageMagick knows by name, without fontconfig.

    type-xml.py FONTDIR OUTPUT

FONTDIR holds dejavu/ and nanum/ (the guest's /usr/share/fonts; build/fonts for the native
reference). Each font gets the name fontconfig would give it in ImageMagick (family and style,
dashes for spaces: DejaVu-Sans-Bold-Oblique), its family, weight, style and stretch (for -family
-weight -style -stretch); then the names scripts use for the standard PostScript, Windows and
generic fonts, as the DejaVu face closest to each.
"""
import sys

SANS = [("", "Book", 400, "normal", ""), ("-Bold", "Bold", 700, "normal", "-Bold"),
        ("-Oblique", "Oblique", 400, "oblique", "-Oblique"),
        ("-Bold-Oblique", "Bold Oblique", 700, "oblique", "-BoldOblique")]
SERIF = [("", "Book", 400, "normal", ""), ("-Bold", "Bold", 700, "normal", "-Bold"),
         ("-Italic", "Italic", 400, "italic", "-Italic"),
         ("-Bold-Italic", "Bold Italic", 700, "italic", "-BoldItalic")]

# alias, family, DejaVu file base, [(name suffix, fullname suffix, weight, style, file suffix)]
ALIASES = [
    ("Helvetica", "Helvetica", "DejaVuSans", [("", "", 400, "normal", ""), ("-Bold", " Bold", 700, "normal", "-Bold"),
        ("-Oblique", " Oblique", 400, "oblique", "-Oblique"), ("-BoldOblique", " Bold Oblique", 700, "oblique", "-BoldOblique")]),
    ("Arial", "Arial", "DejaVuSans", [("", "", 400, "normal", ""), ("-Bold", " Bold", 700, "normal", "-Bold"),
        ("-Italic", " Italic", 400, "italic", "-Oblique"), ("-Bold-Italic", " Bold Italic", 700, "italic", "-BoldOblique")]),
    ("Sans", "Sans", "DejaVuSans", [("", "", 400, "normal", ""), ("-Bold", " Bold", 700, "normal", "-Bold"),
        ("-Oblique", " Oblique", 400, "oblique", "-Oblique"), ("-Bold-Oblique", " Bold Oblique", 700, "oblique", "-BoldOblique")]),
    ("Times", "Times", "DejaVuSerif", [("-Roman", " Roman", 400, "normal", ""), ("-Bold", " Bold", 700, "normal", "-Bold"),
        ("-Italic", " Italic", 400, "italic", "-Italic"), ("-BoldItalic", " Bold Italic", 700, "italic", "-BoldItalic")]),
    ("Times-New-Roman", "Times New Roman", "DejaVuSerif", [("", "", 400, "normal", ""), ("-Bold", " Bold", 700, "normal", "-Bold"),
        ("-Italic", " Italic", 400, "italic", "-Italic"), ("-Bold-Italic", " Bold Italic", 700, "italic", "-BoldItalic")]),
    ("Serif", "Serif", "DejaVuSerif", [("", "", 400, "normal", ""), ("-Bold", " Bold", 700, "normal", "-Bold"),
        ("-Italic", " Italic", 400, "italic", "-Italic"), ("-Bold-Italic", " Bold Italic", 700, "italic", "-BoldItalic")]),
    ("Courier", "Courier", "DejaVuSansMono", [("", "", 400, "normal", ""), ("-Bold", " Bold", 700, "normal", "-Bold"),
        ("-Oblique", " Oblique", 400, "oblique", "-Oblique"), ("-BoldOblique", " Bold Oblique", 700, "oblique", "-BoldOblique")]),
    ("Courier-New", "Courier New", "DejaVuSansMono", [("", "", 400, "normal", ""), ("-Bold", " Bold", 700, "normal", "-Bold"),
        ("-Italic", " Italic", 400, "italic", "-Oblique"), ("-Bold-Italic", " Bold Italic", 700, "italic", "-BoldOblique")]),
    ("Monospace", "Monospace", "DejaVuSansMono", [("", "", 400, "normal", ""), ("-Bold", " Bold", 700, "normal", "-Bold"),
        ("-Oblique", " Oblique", 400, "oblique", "-Oblique"), ("-Bold-Oblique", " Bold Oblique", 700, "oblique", "-BoldOblique")]),
]

HEADER = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE typemap [
  <!ELEMENT typemap (type)+>
  <!ATTLIST typemap xmlns CDATA #FIXED ''>
  <!ELEMENT type EMPTY>
  <!ATTLIST type xmlns CDATA #FIXED '' encoding NMTOKEN #IMPLIED
    family CDATA #REQUIRED format NMTOKEN #REQUIRED foundry NMTOKEN #REQUIRED
    fullname CDATA #REQUIRED glyphs CDATA #REQUIRED metrics CDATA #REQUIRED
    name NMTOKEN #REQUIRED stretch NMTOKEN #REQUIRED style NMTOKEN #REQUIRED
    version CDATA #IMPLIED weight CDATA #REQUIRED>
]>
<!--
  The fonts ImageMagick knows by name (-font NAME; -family FAMILY with -weight, -style, -stretch),
  as the imagemagick add-on of collaboCore installs them, without fontconfig:

    DejaVu Sans, Sans Mono, Serif (and Condensed)    dejavu/: Latin, Greek, Cyrillic, symbols
    NanumGothic, NanumMyeongjo                       nanum/: Korean (Hangul, Hanja), with Latin

  and the names scripts use for the standard fonts, as the DejaVu face closest to each: Helvetica,
  Arial and Sans are DejaVu Sans; Times, Times New Roman and Serif DejaVu Serif; Courier, Courier
  New and Monospace DejaVu Sans Mono. Text without a font is in Helvetica (DejaVu Sans); what a
  font has no glyph for is drawn with a fallback font (policy.xml: NanumGothic, then DejaVu Sans).
  A font file can always be given by its path: -font /path/to/font.ttf.
  Made by addons/imagemagick/tools/type-xml.py.
-->
<typemap>
"""


def fonts(root):
    d, n = root + "/dejavu/", root + "/nanum/"
    for family, base, styles, stretch in [("DejaVu Sans", "DejaVuSans", SANS + [("-ExtraLight", "ExtraLight", 200, "normal", "-ExtraLight")], "normal"),
                                          ("DejaVu Sans Mono", "DejaVuSansMono", SANS, "normal"),
                                          ("DejaVu Serif", "DejaVuSerif", SERIF, "normal"),
                                          ("DejaVu Sans", "DejaVuSansCondensed", SANS, "condensed"),
                                          ("DejaVu Serif", "DejaVuSerifCondensed", SERIF, "condensed")]:
        for suffix, full, weight, style, file_suffix in styles:
            cond = "-Condensed" if stretch == "condensed" else ""
            fullname = family + (" Condensed" if cond else "") + ("" if (cond and full == "Book") else " " + full)
            yield (family.replace(" ", "-") + cond + suffix, fullname, family, weight, style, stretch,
                   d + base + file_suffix + ".ttf")
    yield ("DejaVu-Math-TeX-Gyre", "DejaVu Math TeX Gyre", "DejaVu Math TeX Gyre", 400, "normal", "normal",
           d + "DejaVuMathTeXGyre.ttf")
    for family in ("NanumGothic", "NanumMyeongjo"):
        yield (family, family + " Regular", family, 400, "normal", "normal", n + family + "-Regular.ttf")
        yield (family + "-Bold", family + " Bold", family, 700, "normal", "normal", n + family + "-Bold.ttf")
    for alias, family, base, styles in ALIASES:
        for suffix, full, weight, style, file_suffix in styles:
            yield (alias + suffix, family + full, family, weight, style, "normal", d + base + file_suffix + ".ttf")


def main():
    root, output = sys.argv[1].rstrip("/"), sys.argv[2]
    entries = list(fonts(root))
    names = [e[0] for e in entries]
    assert len(names) == len(set(names)), "a name twice"
    lines = [HEADER]
    for name, full, family, weight, style, stretch, path in entries:
        lines.append(f'  <type name="{name}" fullname="{full}" family="{family}" foundry="collabo" weight="{weight}" '
                     f'style="{style}" stretch="{stretch}" format="truetype" glyphs="{path}"/>\n')
    lines.append("</typemap>\n")
    with open(output, "w", encoding="utf-8") as f:
        f.write("".join(lines))


if __name__ == "__main__":
    main()

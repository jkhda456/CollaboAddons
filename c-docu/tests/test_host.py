#!/usr/bin/env python3
"""docu built natively: that edits are partial (the package's other parts kept byte for byte, the
edited part changed only where the edit is), that what it writes opens in LibreOffice with the
change (when LibreOffice is there), the old binary formats, conversions and rendering. Run by
tests/run.sh; see common.py for the environment variables.
"""
import csv
import io
import os
import re
import shutil
import sys
import zipfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import (HOST, check, emf, fixtures, host, lo_convert, png_pixels, skip, soffice,  # noqa: E402
                    summary, workdir, zip_entries)


def read_member(path, name):
    with zipfile.ZipFile(path) as z:
        return z.read(name).decode()


def changed_parts(before, after):
    a, b = zip_entries(before), zip_entries(after)
    return sorted(n for n in set(a) | set(b) if a.get(n) != b.get(n))


def only_within(old, new, tag):
    """Whether old and new XML differ only inside one element `tag` of old (by its text)."""
    i = 0
    while i < min(len(old), len(new)) and old[i] == new[i]:
        i += 1
    j = 0
    while j < min(len(old), len(new)) - i and old[-1 - j] == new[-1 - j]:
        j += 1
    start = old.rfind(f"<{tag}", 0, i + 1)
    end = old.find(f"</{tag}>", len(old) - j - 1)
    if start < 0 or end < 0:
        return False
    return old.find(f"</{tag}>", start) == end


def main():
    if not os.path.exists(HOST):
        print(f"missing {HOST} (cargo build --target-dir build/host)", file=sys.stderr)
        return 1
    w = workdir("docu-host-")
    fixtures(w)
    orig = os.path.join(w, "orig")
    os.mkdir(orig)
    for n in os.listdir(w):
        if os.path.isfile(os.path.join(w, n)):
            shutil.copyfile(os.path.join(w, n), os.path.join(orig, n))
    o = lambda n: os.path.join(orig, n)  # noqa: E731
    f = lambda n: os.path.join(w, n)  # noqa: E731

    # ── Word: partial edits ────────────────────────────────────────────────────────────────
    st, out = host(["replace", "report.docx", "lazy dog", "sleepy cat"], w)
    check("docx replace", st == 0 and "replaced 1 occurrence in p3" in out, out)
    check("docx replace: only document.xml changes", changed_parts(o("report.docx"), f("report.docx")) == ["word/document.xml"],
          changed_parts(o("report.docx"), f("report.docx")))
    old, new = read_member(o("report.docx"), "word/document.xml"), read_member(f("report.docx"), "word/document.xml")
    check("docx replace: only inside the paragraph", only_within(old, new, "w:p"))
    shutil.copyfile(f("report.docx"), f("step1.docx"))
    st, out = host(["style", "report.docx", "p3", "bold", "color=#C00000", "--match", "brown fox"], w)
    check("docx style --match", st == 0 and "styled p3" in out, out)
    check("docx style: styles.xml kept", changed_parts(f("step1.docx"), f("report.docx")) == ["word/document.xml"])
    st, out = host(["read", "report.docx", "p3", "--runs"], w)
    check("docx style: a run of its own", re.search(r'"brown fox" bold color=C00000', out) is not None, out)
    st, out = host(["replace", "report.docx", "Revenue", "Sales", "--in", "hdr1"], w)
    check("replace --in with no match fails", st != 0 and "nothing written" in out, out)
    st, out = host(["replace", "report.docx", "2023", "2024", "--expect", "3"], w)
    check("replace --expect fails on a count mismatch", st != 0 and "expected 3" in out, out)
    st, out = host(["replace", "report.docx", "2023", "2024", "--expect", "8"], w)
    check("replace across stories", st == 0 and "hdr1.p1" in out and "fn1.p1" in out, out)
    st, out = host(["insert", "report.docx", "p2", "Overview", "style=Heading 2"], w)
    st2, out2 = host(["read", "report.docx", "p3"], w)
    check("docx insert with a style", st == 0 and "[heading 2]: Overview" in out2, out + out2)
    st, out = host(["insert", "report.docx", "p3", "One\n\nTwo"], w)
    check("insert: one paragraph per line, blank lines dropped", st == 0 and "inserted p4, p5" in out, out)
    st, out = host(["style", "report.docx", "p4-5", "list=number"], w)
    st2, out2 = host(["read", "report.docx", "p4-5"], w)
    check("docx list=number", st == 0 and out2.count("{list=") == 2, out + out2)
    st, out = host(["table", "report.docx", "p5", "A|B;1|2", "--header"], w)
    check("docx table", st == 0 and "inserted table t1" in out, out)
    st, out = host(["image", "report.docx", "p1", "pic.png", "w=2cm", "alt=Logo"], w)
    check("docx image", st == 0 and "inserted a picture (2.0x1.0cm) in p2" in out, out)
    names = zipfile.ZipFile(f("report.docx")).namelist()
    check("docx image: a media part", any(n.startswith("word/media/image") for n in names), names)
    st, out = host(["set", "report.docx", "t2.r3.c3", "x"], w)
    check("docx set in a merged cell warns", st == 0 and "vertical merge" in out, out)

    # ── Excel ──────────────────────────────────────────────────────────────────────────────
    st, out = host(["set", "book.xlsx", "Summary!B2", "1500"], w)
    check("xlsx set", st == 0, out)
    parts = changed_parts(o("book.xlsx"), f("book.xlsx"))
    check("xlsx set: the sheet, the workbook (recalc flags), no shared strings", "xl/worksheets/sheet1.xml" in parts
          and "xl/sharedStrings.xml" not in parts and "xl/styles.xml" not in parts, parts)
    st, out = host(["read", "book.xlsx", "Summary!D2"], w)
    check("xlsx: formulas recalculated on save", "{value=2850.5}" in out, out)
    st, out = host(["style", "book.xlsx", "Summary!B2:C4", "numfmt=#,##0", "bold"], w)
    st2, out2 = host(["read", "book.xlsx", "Summary!B2"], w)
    check("xlsx style numfmt", st == 0 and out2.rstrip().endswith(": 1,500"), out + out2)
    # A row between Revenue (2) and Cost (3): Profit's =B2-B3 becomes =B2-B4 on row 5.
    st, out = host(["insert", "book.xlsx", "Summary!2:2", "Rent|300|300"], w)
    st2, out2 = host(["read", "book.xlsx", "Summary!A3:B5"], w)
    check("xlsx insert rows moves formulas", st == 0 and "Summary!A3: Rent" in out2 and "{=B2-B4}" in out2, out + out2)
    st, out = host(["set", "book.xlsx", "Summary!G1", "Year|Amount;2024|=B2*2"], w)
    st2, out2 = host(["read", "book.xlsx", "Summary!H2"], w)
    check("xlsx set a block with a formula", st == 0 and "{=B2*2}" in out2 and "3000" in out2, out + out2)
    if soffice():
        lo = os.path.join(w, "lo")
        os.mkdir(lo)
        c = lo_convert(f("book.xlsx"), "csv", lo)
        rows = list(csv.reader(io.open(c, encoding="utf-8", errors="replace"))) if c else []
        check("LibreOffice opens the xlsx with the edits", any(r[:1] == ["Rent"] for r in rows) and any("3000" in "".join(r) for r in rows), rows[:8])
    else:
        skip("LibreOffice checks", "no soffice")

    # ── PowerPoint ─────────────────────────────────────────────────────────────────────────
    st, out = host(["replace", "deck.pptx", "Highlights", "Key points"], w)
    check("pptx replace: only the slide changes", st == 0 and changed_parts(o("deck.pptx"), f("deck.pptx")) == ["ppt/slides/slide2.xml"],
          changed_parts(o("deck.pptx"), f("deck.pptx")))
    st, out = host(["slide", "deck.pptx", "add", "--layout", "Title and Content", "--title", "Next"], w)
    st2, out2 = host(["read", "deck.pptx", "s3"], w)
    check("pptx slide add", st == 0 and "s3.sh1.p1: Next" in out2, out + out2)
    st, out = host(["set", "deck.pptx", "s3.sh2", "A\nB\nC"], w)
    st2, out2 = host(["read", "deck.pptx", "s3.sh2"], w)
    check("pptx set a body: a paragraph per line", st == 0 and "s3.sh2.p3: C" in out2, out + out2)
    st, out = host(["image", "deck.pptx", "s2.sh5", "pic.png"], w)
    st2, out2 = host(["read", "deck.pptx", "s2.sh5"], w)
    check("pptx replace a picture, fitted in its box", st == 0 and "2.8x1.4cm" in out2, out + out2)
    st, out = host(["read", "deck.pptx", "s2.sh[Title 1]"], w)
    check("pptx shape by name", st == 0 and "Key points" in out, out)

    # ── charts ─────────────────────────────────────────────────────────────────────────────
    st, out = host(["chart", "charts.pptx", "s1.sh2"], w)
    check("chart: show", st == 0 and 'series 1 "Sales": 4.3 | 2.5 | 3.5' in out and "(Sheet1!$B$2:$B$4)" in out, out)
    st, out = host(["chart", "charts.pptx", "s1.sh2", "data", "Quarter|Sales|Cost;Q1|8|5;Q2|3|2;Q3|9|4;Q4|7|3"], w)
    check("chart: new data, its data sheet too", st == 0 and "2 series × 4 categories" in out and "data sheet" in out, out)
    st, out = host(["chart", "charts.pptx", "s1.sh2", "title", "Sales and cost"], w)
    st2, out2 = host(["chart", "charts.pptx", "s1.sh2"], w)
    check("chart: title", st == 0 and 'title "Sales and cost"' in out2 and 'series 2 "Cost": 5 | 2 | 4 | 3   (Sheet1!$C$2:$C$5)' in out2, out + out2)
    changed = changed_parts(o("charts.pptx"), f("charts.pptx"))
    check("chart edit: only the chart and its workbook change", changed == ["ppt/charts/chart1.xml", "ppt/embeddings/Microsoft_Excel_Worksheet.xlsx"], changed)
    with zipfile.ZipFile(f("charts.pptx")) as z:
        emb = z.read("ppt/embeddings/Microsoft_Excel_Worksheet.xlsx")
    with open(f("emb.xlsx"), "wb") as e:
        e.write(emb)
    st, out = host(["read", "emb.xlsx"], w)
    table = read_member(f("emb.xlsx"), "xl/tables/table1.xml")
    check("chart: the data sheet and its table", "Sheet1!C5: 3" in out and 'ref="A1:C5"' in table and 'name="Cost"' in table, out + table)
    if soffice():
        lo = os.path.join(w, "lo3")
        os.mkdir(lo)
        p = lo_convert(f("charts.pptx"), "odp", lo)
        content = read_member(p, "Object 1/content.xml") if p and "Object 1/content.xml" in zipfile.ZipFile(p).namelist() else ""
        check("LibreOffice reads the edited chart", "Cost" in content and "Q4" in content, content[:300] or p)

    # ── the old formats ────────────────────────────────────────────────────────────────────
    st, out = host(["replace", "report.doc", "lazy dog", "sleepy cat"], w)
    check(".doc replace", st == 0 and "p3" in out, out)
    st, out = host(["insert", "report.doc", "p3", "Inserted paragraph."], w)
    st2, out2 = host(["read", "report.doc", "p3-4"], w)
    check(".doc insert", st == 0 and "p4: Inserted paragraph." in out2 and "sleepy cat" in out2, out + out2)
    st, out = host(["set", "book.xls", "Summary!B2", "1500"], w)
    st2, out2 = host(["read", "book.xls", "Summary!D2"], w)
    check(".xls set recalculates", st == 0 and "2850.5" in out2.replace(",", ""), out + out2)
    st, out = host(["replace", "deck.ppt", "Highlights", "Key points"], w)
    st2, out2 = host(["read", "deck.ppt", "s2"], w)
    check(".ppt replace", st == 0 and "Key points of 2023" in out2, out + out2)
    if soffice():
        lo = os.path.join(w, "lo2")
        os.mkdir(lo)
        t = lo_convert(f("report.doc"), "txt:Text (encoded):UTF8", lo)
        text = open(t, encoding="utf-8", errors="replace").read() if t else ""
        check("LibreOffice reads the edited .doc", "sleepy cat" in text and "Inserted paragraph." in text, text[:600])
        c = lo_convert(f("book.xls"), "csv", lo)
        text = open(c, encoding="utf-8", errors="replace").read() if c else ""
        check("LibreOffice reads the edited .xls", "1500" in text.replace(",", ""), text[:400])
        p = lo_convert(f("deck.ppt"), "pptx", lo)
        text = read_member(p, "ppt/slides/slide2.xml") if p else ""
        check("LibreOffice reads the edited .ppt", "Key points" in text, text[:400])

    # ── new, convert, render ───────────────────────────────────────────────────────────────
    for name, text in (("n.docx", "Plan"), ("n.xlsx", "1"), ("n.pptx", "Deck")):
        st, out = host(["new", name, text], w)
        st2, out2 = host(["read", name], w)
        check(f"new {name}", st == 0 and (text in out2), out + out2)
    st, out = host(["new", "n.docx"], w)
    check("new refuses to overwrite", st != 0 and "--force" in out, out)
    for src, dst, want in (("report.doc", "c.docx", "sleepy cat"), ("book.xls", "c.xlsx", "Revenue"), ("deck.ppt", "c.pptx", "Key points"),
                           ("report.docx", "c.md", "# Annual Report"), ("book.xlsx", "c.csv", "Rent")):
        st, out = host(["convert", src, dst], w)
        st2, out2 = (0, open(f(dst), encoding="utf-8").read()) if dst.endswith((".md", ".csv")) else host(["text", dst], w)
        check(f"convert {src} → {dst}", st == 0 and want in out2, out + out2[:500])
    for src in ("report.docx", "book.xlsx", "deck.pptx", "report.doc", "book.xls", "deck.ppt"):
        st, out = host(["render", src, "-o", f"r-{src}.png", "--ppi", "30"], w)
        pages = [n for n in os.listdir(w) if n.startswith(f"r-{src}") and n.endswith(".png")]
        check(f"render {src}", st == 0 and pages and all(os.path.getsize(f(n)) > 200 for n in pages), out)
    # A metafile picture is drawn (as SVG): its rectangle in its brush's color.
    emf(f("box.emf"))
    host(["new", "m.docx"], w)
    st, out = host(["image", "m.docx", "p1", "box.emf"], w)
    st2, out2 = host(["render", "m.docx", "-o", "m.png", "--ppi", "96"], w)
    wpx, hpx, rows = png_pixels(f("m.png"))
    green = sum(1 for r in rows for px in r if abs(px[0] - 0x20) < 8 and abs(px[1] - 0xA0) < 8 and abs(px[2] - 0x40) < 8)
    check("an EMF picture is drawn", st == 0 and st2 == 0 and "2.6x2.6cm" in out and 5000 < green < 8000, (out, out2, green))
    st, out = host(["render", "report.docx", "-o", "r.pdf"], w)
    check("render PDF", st == 0 and open(f("r.pdf"), "rb").read(5) == b"%PDF-", out)
    return summary()


if __name__ == "__main__":
    sys.exit(main())

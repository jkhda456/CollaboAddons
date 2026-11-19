#!/bin/sh
# The same edits in the guest and natively (test_guest.py compares what they write, byte for
# byte): run in a folder holding report.docx, book.xlsx, deck.pptx (tests/fixtures.py), report.doc,
# book.xls, deck.ppt (tests/legacy) and pic.png; DOCU names the program (default: docu).
# Each command's output goes to log.txt; files are edited in place or written as out-*.
set -u
D="${DOCU:-docu}"
run() {
    echo "\$ docu $*" >> log.txt
    "$D" "$@" >> log.txt 2>&1
    echo "exit $?" >> log.txt
}

# Word
run read report.docx
run read report.docx p3 t1.r2 hdr1 fn1 --runs
run find report.docx 2023
run replace report.docx 2023 2024 --expect 8
run replace report.docx "Kim Chulsoo" "박영희"
run style report.docx p3 bold color=#C00000 --match "lazy dog"
run style report.docx p2 style="Heading 2"
run insert report.docx p3 "A new paragraph.

And another one." align=justify
run insert report.docx p5 "First item" list=bullet
run table report.docx p6 "Item|Q1|Q2;Rent|300|310" --header
run image report.docx p1 pic.png w=3cm alt=Logo
run set report.docx t2.r3.c3 "+2%"
run delete report.docx p4[0:2]
run replace report.docx nothing-like-this x
run read report.docx --json
run text report.docx

# Excel
run read book.xlsx 'Summary!A1:E7'
run set book.xlsx 'Summary!B2' 1500
run set book.xlsx 'Summary!G1' 'Year|Amount;2024|=B2*2'
run style book.xlsx 'Summary!B2:D4' numfmt='#,##0' fill=#DDEBF7 bold
run insert book.xlsx 'Summary!3:3' 'Rent|300|300'
run delete book.xlsx 'Summary!C:C'
run sheet book.xlsx add Notes
run sheet book.xlsx rename Data Raw
run image book.xlsx 'Summary!H2' pic.png w=2cm
run read book.xlsx
run recalc book.xlsx

# PowerPoint
run read deck.pptx
run replace deck.pptx 2023 2024
run style deck.pptx s2.sh2.p1 italic --match grew
run image deck.pptx s2.sh5 pic.png
run set deck.pptx s2.notes "Notes changed"
run slide deck.pptx add --layout "Title and Content" --title "Next steps"
run set deck.pptx s3.sh2 "Hire
Ship
Sell"
run style deck.pptx s3.sh2.p2 level=2
run slide deck.pptx dup 1
run slide deck.pptx move 4 2
run textbox deck.pptx s1 "Draft" x=1cm y=1cm w=4cm h=1.5cm size=20pt color=red
run read deck.pptx 's3.sh[Title 1]'
run read deck.pptx

# Charts (a deck with one made by fixtures.py)
run chart charts.pptx s1.sh2
run chart charts.pptx s1.sh2 data "Quarter|Sales|Cost;Q1|8|5;Q2|3|2;Q3|9|4"
run chart charts.pptx s1.sh2 title "Sales and cost"
run chart charts.pptx s1.sh2
run render charts.pptx -o out-chart.png --ppi 48

# The old formats
run read report.doc
run replace report.doc 2023 2025
run style report.doc p3 italic --match fox
run insert report.doc p3 "Inserted in a .doc."
run read report.doc
run read book.xls
run set book.xls 'Summary!B2' 999
run replace book.xls Revenue Sales
run read book.xls
run read deck.ppt
run replace deck.ppt 2023 2025
run style deck.ppt s1.sh1.p1 color=blue
run read deck.ppt

# New documents
run new new.docx "Plan"
run insert new.docx p1 "Goals" style="Heading 1"
run new new.xlsx
run set new.xlsx A1 "a|b;1|=A2+1"
run new new.pptx "Deck" "Sub"
run read new.xlsx

# Converting and rendering
run convert report.doc out-report.docx
run convert book.xls out-book.xlsx
run convert deck.ppt out-deck.pptx
run convert report.docx out-report.md
run convert book.xlsx out-summary.csv --sheet Summary
run render report.docx -o out-report.png --ppi 48
run render book.xlsx -o out-book.pdf
run render deck.pptx -o out-deck.svg --pages 2
run render report.doc -o out-doc.png --ppi 48
run render deck.ppt -o out-ppt.png --ppi 48 --pages 1
run render out-report.docx -o out-report2.pdf

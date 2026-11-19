"""Test documents made by hand, shaped as Word, Excel and PowerPoint write them (rsids, proofing
marks, runs split mid-word, fields, hyperlinks, text boxes with fallbacks, headers, footnotes,
comments, merged cells), so the checks see what real files hold.

    python3 tests/fixtures.py OUTDIR
"""
import os
import sys
import zipfile

W_NS = ('xmlns:wpc="http://schemas.microsoft.com/office/word/2010/wordprocessingCanvas" '
        'xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" '
        'xmlns:o="urn:schemas-microsoft-com:office:office" '
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
        'xmlns:m="http://schemas.openxmlformats.org/officeDocument/2006/math" '
        'xmlns:v="urn:schemas-microsoft-com:vml" '
        'xmlns:wp14="http://schemas.microsoft.com/office/word/2010/wordprocessingDrawing" '
        'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" '
        'xmlns:w10="urn:schemas-microsoft-com:office:word" '
        'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" '
        'xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" '
        'xmlns:wpg="http://schemas.microsoft.com/office/word/2010/wordprocessingGroup" '
        'xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" '
        'mc:Ignorable="w14 wp14"')

HDR = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'

DOCX_BODY = """<w:body>
<w:p w:rsidR="00A1" w:rsidRDefault="00B2"><w:pPr><w:pStyle w:val="Title"/></w:pPr><w:r><w:t>Annual Report 2023</w:t></w:r></w:p>
<w:p w:rsidR="00A1"><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:bookmarkStart w:id="0" w:name="_Toc1"/><w:r><w:t>Intro</w:t></w:r><w:r w:rsidR="00C3"><w:t>duction</w:t></w:r><w:bookmarkEnd w:id="0"/></w:p>
<w:p w:rsidR="00A1"><w:r><w:t xml:space="preserve">The </w:t></w:r><w:proofErr w:type="spellStart"/><w:r><w:rPr><w:b/></w:rPr><w:t>quick</w:t></w:r><w:proofErr w:type="spellEnd"/><w:r><w:t xml:space="preserve"> brown fox jumps over the lazy dog in 2023.</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Visit </w:t></w:r><w:hyperlink r:id="rId9" w:history="1"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr><w:t>our site</w:t></w:r></w:hyperlink><w:r><w:t xml:space="preserve"> for more.</w:t></w:r><w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteReference w:id="1"/></w:r></w:p>
<w:p><w:r><w:t>Name:</w:t></w:r><w:r><w:tab/></w:r><w:r><w:rPr><w:i/><w:color w:val="1F3864" w:themeColor="accent1" w:themeShade="80"/></w:rPr><w:t>Kim</w:t></w:r><w:r><w:rPr><w:i/></w:rPr><w:t xml:space="preserve"> Chulsoo</w:t></w:r><w:r><w:br/><w:t>Line two</w:t></w:r></w:p>
<w:p><w:r><w:t xml:space="preserve">Page </w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t xml:space="preserve"> of the 2023 report</w:t></w:r></w:p>
<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/></w:tblPr><w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>
<w:tr><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Item</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>2023</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:rPr><w:b/></w:rPr><w:t>Change</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>Revenue</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>1,200</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:vMerge w:val="restart"/></w:tcPr><w:p><w:r><w:t>+8%</w:t></w:r></w:p></w:tc></w:tr>
<w:tr><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>Cost</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/></w:tcPr><w:p><w:r><w:t>700</w:t></w:r></w:p></w:tc><w:tc><w:tcPr><w:tcW w:w="3000" w:type="dxa"/><w:vMerge/></w:tcPr><w:p/></w:tc></w:tr>
</w:tbl>
<w:p><w:r><w:t>한국어 문단입니다. 매출은 2023년에 증가했습니다.</w:t></w:r></w:p>
<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" relativeHeight="1" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1"><wp:simplePos x="0" y="0"/><wp:positionH relativeFrom="column"><wp:posOffset>0</wp:posOffset></wp:positionH><wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV><wp:extent cx="1828800" cy="457200"/><wp:effectExtent l="0" t="0" r="0" b="0"/><wp:wrapSquare wrapText="bothSides"/><wp:docPr id="1" name="Text Box 1"/><wp:cNvGraphicFramePr/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData uri="http://schemas.microsoft.com/office/word/2010/wordprocessingShape"><wps:wsp><wps:cNvSpPr txBox="1"/><wps:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="1828800" cy="457200"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></wps:spPr><wps:txbx><w:txbxContent><w:p><w:r><w:t>Boxed 2023 note</w:t></w:r></w:p></w:txbxContent></wps:txbx><wps:bodyPr/></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice><mc:Fallback><w:pict><v:shape id="Text Box 1" type="#_x0000_t202" style="width:2in;height:36pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>Boxed 2023 note</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r><w:r><w:t>After the box.</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>First point</w:t></w:r></w:p>
<w:p><w:pPr><w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr></w:pPr><w:r><w:t>Second point</w:t></w:r></w:p>
<w:p><w:ins w:id="5" w:author="Lee" w:date="2024-01-01T00:00:00Z"><w:r><w:t>Inserted text. </w:t></w:r></w:ins><w:del w:id="6" w:author="Lee" w:date="2024-01-01T00:00:00Z"><w:r><w:delText>Deleted 2023 text.</w:delText></w:r></w:del><w:commentRangeStart w:id="0"/><w:r><w:t>Commented words</w:t></w:r><w:commentRangeEnd w:id="0"/><w:r><w:rPr><w:rStyle w:val="CommentReference"/></w:rPr><w:commentReference w:id="0"/></w:r></w:p>
<w:sectPr w:rsidR="00A1"><w:headerReference w:type="default" r:id="rId10"/><w:footerReference w:type="default" r:id="rId11"/><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="851" w:footer="992" w:gutter="0"/><w:cols w:space="425"/><w:docGrid w:linePitch="360"/></w:sectPr>
</w:body>"""

STYLES = """<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:asciiTheme="minorHAnsi" w:eastAsiaTheme="minorEastAsia" w:hAnsiTheme="minorHAnsi" w:cstheme="minorBidi"/><w:kern w:val="2"/><w:sz w:val="22"/><w:szCs w:val="24"/><w:lang w:val="en-US" w:eastAsia="ko-KR" w:bidi="ar-SA"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/><w:contextualSpacing/></w:pPr><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:eastAsiaTheme="majorEastAsia" w:hAnsiTheme="majorHAnsi" w:cstheme="majorBidi"/><w:spacing w:val="-10"/><w:kern w:val="28"/><w:sz w:val="56"/><w:szCs w:val="56"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="240" w:after="0"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:rFonts w:asciiTheme="majorHAnsi" w:eastAsiaTheme="majorEastAsia" w:hAnsiTheme="majorHAnsi" w:cstheme="majorBidi"/><w:color w:val="2F5496" w:themeColor="accent1" w:themeShade="BF"/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="40" w:after="0"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:color w:val="2F5496"/><w:sz w:val="26"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720"/><w:contextualSpacing/></w:pPr></w:style>
<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/></w:style>
<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:b/><w:bCs/></w:rPr></w:style>
<w:style w:type="character" w:styleId="FootnoteReference"><w:name w:val="footnote reference"/><w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="CommentReference"><w:name w:val="annotation reference"/><w:rPr><w:sz w:val="18"/></w:rPr></w:style>
<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:tblPr><w:tblInd w:w="0" w:type="dxa"/><w:tblCellMar><w:top w:w="0" w:type="dxa"/><w:left w:w="108" w:type="dxa"/><w:bottom w:w="0" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>
<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:left w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:right w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="auto"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="auto"/></w:tblBorders></w:tblPr></w:style>
</w:styles>"""

NUMBERING = """<w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="decimal"/><w:lvlText w:val="%1."/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl><w:lvl w:ilvl="1"><w:start w:val="1"/><w:numFmt w:val="lowerLetter"/><w:lvlText w:val="%2)"/><w:lvlJc w:val="left"/><w:pPr><w:ind w:left="1440" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum><w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num></w:numbering>"""


def docx(path):
    files = {
        "[Content_Types].xml": HDR + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/><Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/><Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/><Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/><Override PartName="/word/comments.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>',
        "_rels/.rels": HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>',
        "word/_rels/document.xml.rels": HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/><Relationship Id="rId9" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" Target="https://example.com/" TargetMode="External"/><Relationship Id="rId10" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/><Relationship Id="rId11" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/><Relationship Id="rId12" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/><Relationship Id="rId13" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/comments" Target="comments.xml"/></Relationships>',
        "word/document.xml": HDR + f'<w:document {W_NS}>' + DOCX_BODY.replace("\n", "") + '</w:document>',
        "word/styles.xml": HDR + STYLES.replace("\n", ""),
        "word/numbering.xml": HDR + NUMBERING,
        "word/header1.xml": HDR + f'<w:hdr {W_NS}><w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>ACME Corp — Confidential 2023</w:t></w:r></w:p></w:hdr>',
        "word/footer1.xml": HDR + f'<w:ftr {W_NS}><w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t xml:space="preserve">Page </w:t></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>1</w:t></w:r></w:fldSimple></w:p></w:ftr>',
        "word/footnotes.xml": HDR + f'<w:footnotes {W_NS}><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote><w:footnote w:id="1"><w:p><w:pPr><w:pStyle w:val="FootnoteText"/></w:pPr><w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteRef/></w:r><w:r><w:t xml:space="preserve"> Source: 2023 filings.</w:t></w:r></w:p></w:footnote></w:footnotes>',
        "word/comments.xml": HDR + f'<w:comments {W_NS}><w:comment w:id="0" w:author="Park" w:date="2024-02-01T00:00:00Z" w:initials="P"><w:p><w:r><w:t>Check the 2023 numbers.</w:t></w:r></w:p></w:comment></w:comments>',
        "docProps/core.xml": HDR + '<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>Report</dc:title><dc:creator>Kim</dc:creator></cp:coreProperties>',
    }
    write(path, files)


def write(path, files):
    with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as z:
        for name, data in files.items():
            z.writestr(name, data.encode() if isinstance(data, str) else data)


CHART = """<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><c:date1904 val="0"/><c:lang val="en-US"/><c:roundedCorners val="0"/><c:chart><c:autoTitleDeleted val="1"/><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>Sales</c:v></c:pt></c:strCache></c:strRef></c:tx><c:spPr><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></c:spPr><c:invertIfNegative val="0"/><c:cat><c:strRef><c:f>Sheet1!$A$2:$A$4</c:f><c:strCache><c:ptCount val="3"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt><c:pt idx="2"><c:v>Q3</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$B$2:$B$4</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/><c:pt idx="0"><c:v>4.3</c:v></c:pt><c:pt idx="1"><c:v>2.5</c:v></c:pt><c:pt idx="2"><c:v>3.5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:gapWidth val="219"/><c:overlap val="-27"/><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:numFmt formatCode="General" sourceLinked="1"/><c:majorTickMark val="none"/><c:minorTickMark val="none"/><c:tickLblPos val="nextTo"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="1197"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="65000"/><a:lumOff val="35000"/></a:schemeClr></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr><c:crossAx val="2"/><c:crosses val="autoZero"/><c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/><c:noMultiLvlLbl val="0"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines><c:spPr><a:ln w="9525"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="15000"/><a:lumOff val="85000"/></a:schemeClr></a:solidFill></a:ln></c:spPr></c:majorGridlines><c:numFmt formatCode="General" sourceLinked="1"/><c:majorTickMark val="none"/><c:minorTickMark val="none"/><c:tickLblPos val="nextTo"/><c:spPr><a:ln><a:noFill/></a:ln></c:spPr><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="1197"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="65000"/><a:lumOff val="35000"/></a:schemeClr></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr><c:crossAx val="1"/><c:crosses val="autoZero"/><c:crossBetween val="between"/></c:valAx></c:plotArea><c:legend><c:legendPos val="b"/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr/></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr><c:externalData r:id="rId1"><c:autoUpdate val="0"/></c:externalData></c:chartSpace>"""


def chart_workbook():
    """The chart's data sheet, as PowerPoint embeds it: Sheet1 with a table over A1:B4."""
    import io
    buf = io.BytesIO()
    sheet = (f'<worksheet {SML}><dimension ref="A1:B4"/><sheetData>'
             '<row r="1"><c r="B1" t="s"><v>0</v></c></row>'
             '<row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2"><v>4.3</v></c></row>'
             '<row r="3"><c r="A3" t="s"><v>2</v></c><c r="B3"><v>2.5</v></c></row>'
             '<row r="4"><c r="A4" t="s"><v>3</v></c><c r="B4"><v>3.5</v></c></row>'
             '</sheetData><tableParts count="1"><tablePart r:id="rId1"/></tableParts></worksheet>')
    table = ('<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Table1" displayName="Table1" ref="A1:B4" totalsRowShown="0">'
             '<tableColumns count="2"><tableColumn id="1" name=" "/><tableColumn id="2" name="Sales"/></tableColumns>'
             '<tableStyleInfo name="TableStyleMedium2" showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/></table>')
    sst = ('<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="4">'
           '<si><t>Sales</t></si><si><t>Q1</t></si><si><t>Q2</t></si><si><t>Q3</t></si></sst>')
    rel = lambda rid, t, target: f'<Relationship Id="{rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{t}" Target="{target}"/>'
    rels = lambda *r: XL_HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + "".join(r) + '</Relationships>'
    ov = lambda part, ct: f'<Override PartName="{part}" ContentType="application/vnd.openxmlformats-officedocument.{ct}"/>'
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", XL_HDR + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>'
                   + ov("/xl/workbook.xml", "spreadsheetml.sheet.main+xml") + ov("/xl/worksheets/sheet1.xml", "spreadsheetml.worksheet+xml")
                   + ov("/xl/sharedStrings.xml", "spreadsheetml.sharedStrings+xml") + ov("/xl/tables/table1.xml", "spreadsheetml.table+xml") + "</Types>")
        z.writestr("_rels/.rels", rels(rel("rId1", "officeDocument", "xl/workbook.xml")))
        z.writestr("xl/workbook.xml", XL_HDR + f'<workbook {SML}><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>')
        z.writestr("xl/_rels/workbook.xml.rels", rels(rel("rId1", "worksheet", "worksheets/sheet1.xml"), rel("rId2", "sharedStrings", "sharedStrings.xml")))
        z.writestr("xl/worksheets/sheet1.xml", XL_HDR + sheet)
        z.writestr("xl/worksheets/_rels/sheet1.xml.rels", rels(rel("rId1", "table", "../tables/table1.xml")))
        z.writestr("xl/tables/table1.xml", XL_HDR + table)
        z.writestr("xl/sharedStrings.xml", XL_HDR + sst)
    return buf.getvalue()


def charts_pptx(path, deck):
    """deck.pptx's first slide with a column chart (and its data sheet) instead of its subtitle."""
    with zipfile.ZipFile(deck) as z:
        files = {n: z.read(n) for n in z.namelist()}
    frame = ('<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Chart 3"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>'
             '<p:xfrm><a:off x="1524000" y="3602038"/><a:ext cx="9144000" cy="3000000"/></p:xfrm><a:graphic>'
             '<a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId2"/>'
             '</a:graphicData></a:graphic></p:graphicFrame>')
    s1 = files["ppt/slides/slide1.xml"].decode()
    start = s1.find('<p:sp><p:nvSpPr><p:cNvPr id="3"')
    end = s1.find("</p:sp>", start) + len("</p:sp>")
    files["ppt/slides/slide1.xml"] = (s1[:start] + frame + s1[end:]).encode()
    r = files["ppt/slides/_rels/slide1.xml.rels"].decode()
    files["ppt/slides/_rels/slide1.xml.rels"] = r.replace("</Relationships>", '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>').encode()
    files["ppt/charts/chart1.xml"] = (XL_HDR + CHART).encode()
    files["ppt/charts/_rels/chart1.xml.rels"] = (XL_HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="../embeddings/Microsoft_Excel_Worksheet.xlsx"/></Relationships>').encode()
    files["ppt/embeddings/Microsoft_Excel_Worksheet.xlsx"] = chart_workbook()
    ct = files["[Content_Types].xml"].decode()
    files["[Content_Types].xml"] = ct.replace("</Types>", '<Default Extension="xlsx" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"/><Override PartName="/ppt/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/></Types>').encode()
    write(path, {k: v for k, v in files.items()})


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out, exist_ok=True)
    docx(os.path.join(out, "report.docx"))
    print("wrote", out)


XL_HDR = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
SML = 'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x14ac" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac"'


def xlsx(path):
    strings = ["Item", "Q1", "Q2", "Total", "Revenue", "Cost", "Profit", "Date", "Note", "Shared label", "한글 항목"]
    sst = "".join(f"<si><t>{s}</t></si>" for s in strings)
    sst += '<si><r><rPr><b/><sz val="11"/><color rgb="FFFF0000"/><rFont val="Calibri"/></rPr><t>Rich</t></r><r><rPr><sz val="11"/><rFont val="Calibri"/></rPr><t xml:space="preserve"> text 2023</t></r></si>'
    n_si = len(strings) + 1
    sheet1 = f"""<worksheet {SML}><dimension ref="A1:E7"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"><selection activeCell="B2" sqref="B2"/></sheetView></sheetViews><sheetFormatPr defaultRowHeight="15" x14ac:dyDescent="0.25"/><cols><col min="1" max="1" width="14.7109375" customWidth="1"/><col min="2" max="4" width="10" customWidth="1"/></cols><sheetData>
<row r="1" spans="1:5"><c r="A1" s="1" t="s"><v>0</v></c><c r="B1" s="1" t="s"><v>1</v></c><c r="C1" s="1" t="s"><v>2</v></c><c r="D1" s="1" t="s"><v>3</v></c><c r="E1" s="1" t="s"><v>8</v></c></row>
<row r="2" spans="1:5"><c r="A2" t="s"><v>4</v></c><c r="B2" s="2"><v>1200</v></c><c r="C2" s="2"><v>1350.5</v></c><c r="D2" s="2"><f>SUM(B2:C2)</f><v>2550.5</v></c><c r="E2" t="s"><v>9</v></c></row>
<row r="3" spans="1:5"><c r="A3" t="s"><v>5</v></c><c r="B3" s="2"><v>700</v></c><c r="C3" s="2"><v>810</v></c><c r="D3" s="2"><f t="shared" ref="D3:D4" si="0">SUM(B3:C3)</f><v>1510</v></c><c r="E3" t="s"><v>9</v></c></row>
<row r="4" spans="1:5"><c r="A4" t="s"><v>6</v></c><c r="B4" s="2"><f>B2-B3</f><v>500</v></c><c r="C4" s="2"><f>C2-C3</f><v>540.5</v></c><c r="D4" s="2"><f t="shared" si="0"/><v>1040.5</v></c><c r="E4" t="s"><v>{n_si-1}</v></c></row>
<row r="5" spans="1:5"><c r="A5" t="s"><v>7</v></c><c r="B5" s="3"><v>45306</v></c><c r="C5" s="4"><v>0.256</v></c><c r="D5"><f>Rate*D2</f><v>255.05</v></c></row>
<row r="6"><c r="A6" t="s"><v>10</v></c><c r="B6" t="inlineStr"><is><t>inline 2023 text</t></is></c><c r="C6" t="b"><v>1</v></c><c r="D6" t="str"><f>A2&amp;" "&amp;TEXT(D2,"#,##0")</f><v>Revenue 2,551</v></c></row>
<row r="7"><c r="A7" t="s"><v>9</v></c><c r="B7"><f>Data!B2*2</f><v>84</v></c><c r="C7" t="e"><f>1/0</f><v>#DIV/0!</v></c></row>
</sheetData><mergeCells count="1"><mergeCell ref="E2:E3"/></mergeCells><conditionalFormatting sqref="B2:C4"><cfRule type="cellIs" dxfId="0" priority="1" operator="greaterThan"><formula>1000</formula></cfRule></conditionalFormatting><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/></worksheet>"""
    sheet2 = f"""<worksheet {SML}><dimension ref="A1:B2"/><sheetData><row r="1"><c r="A1" t="s"><v>9</v></c><c r="B1"><v>7</v></c></row><row r="2"><c r="A2" t="s"><v>8</v></c><c r="B2"><f>B1*6</f><v>42</v></c></row></sheetData></worksheet>"""
    styles = f"""<styleSheet {SML}><numFmts count="1"><numFmt numFmtId="164" formatCode="yyyy\\-mm\\-dd"/></numFmts><fonts count="2" x14ac:knownFonts="1"><font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font><font><b/><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="5"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/><xf numFmtId="4" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="164" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/><xf numFmtId="10" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles><dxfs count="1"><dxf><font><color rgb="FF9C0006"/></font></dxf></dxfs></styleSheet>"""
    files = {
        "[Content_Types].xml": XL_HDR + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/><Override PartName="/xl/calcChain.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.calcChain+xml"/></Types>',
        "_rels/.rels": XL_HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
        "xl/_rels/workbook.xml.rels": XL_HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/><Relationship Id="rId4" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/><Relationship Id="rId5" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/calcChain" Target="calcChain.xml"/></Relationships>',
        "xl/workbook.xml": XL_HDR + f'<workbook {SML}><workbookPr defaultThemeVersion="164011"/><bookViews><workbookView xWindow="0" yWindow="0" windowWidth="28800" windowHeight="12300"/></bookViews><sheets><sheet name="Summary" sheetId="1" r:id="rId1"/><sheet name="Data" sheetId="2" r:id="rId2"/></sheets><definedNames><definedName name="Rate">0.1</definedName><definedName name="_xlnm.Print_Area" localSheetId="0">Summary!$A$1:$E$7</definedName></definedNames><calcPr calcId="162913"/></workbook>',
        "xl/worksheets/sheet1.xml": XL_HDR + sheet1.replace("\n", ""),
        "xl/worksheets/sheet2.xml": XL_HDR + sheet2,
        "xl/styles.xml": XL_HDR + styles,
        "xl/sharedStrings.xml": XL_HDR + f'<sst {SML.split(" xmlns:r")[0]} count="20" uniqueCount="{n_si}">{sst}</sst>',
        "xl/calcChain.xml": XL_HDR + '<calcChain xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><c r="D2" i="1"/><c r="D3"/><c r="D4"/><c r="B4"/><c r="C4"/><c r="D5"/><c r="D6"/><c r="B7"/><c r="C7"/><c r="B2" i="2"/></calcChain>',
    }
    write(path, files)


CHART = """<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><c:date1904 val="0"/><c:lang val="en-US"/><c:roundedCorners val="0"/><c:chart><c:autoTitleDeleted val="1"/><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>Sales</c:v></c:pt></c:strCache></c:strRef></c:tx><c:spPr><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></c:spPr><c:invertIfNegative val="0"/><c:cat><c:strRef><c:f>Sheet1!$A$2:$A$4</c:f><c:strCache><c:ptCount val="3"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt><c:pt idx="2"><c:v>Q3</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$B$2:$B$4</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/><c:pt idx="0"><c:v>4.3</c:v></c:pt><c:pt idx="1"><c:v>2.5</c:v></c:pt><c:pt idx="2"><c:v>3.5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:gapWidth val="219"/><c:overlap val="-27"/><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:numFmt formatCode="General" sourceLinked="1"/><c:majorTickMark val="none"/><c:minorTickMark val="none"/><c:tickLblPos val="nextTo"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="1197"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="65000"/><a:lumOff val="35000"/></a:schemeClr></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr><c:crossAx val="2"/><c:crosses val="autoZero"/><c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/><c:noMultiLvlLbl val="0"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines><c:spPr><a:ln w="9525"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="15000"/><a:lumOff val="85000"/></a:schemeClr></a:solidFill></a:ln></c:spPr></c:majorGridlines><c:numFmt formatCode="General" sourceLinked="1"/><c:majorTickMark val="none"/><c:minorTickMark val="none"/><c:tickLblPos val="nextTo"/><c:spPr><a:ln><a:noFill/></a:ln></c:spPr><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="1197"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="65000"/><a:lumOff val="35000"/></a:schemeClr></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr><c:crossAx val="1"/><c:crosses val="autoZero"/><c:crossBetween val="between"/></c:valAx></c:plotArea><c:legend><c:legendPos val="b"/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr/></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr><c:externalData r:id="rId1"><c:autoUpdate val="0"/></c:externalData></c:chartSpace>"""


def chart_workbook():
    """The chart's data sheet, as PowerPoint embeds it: Sheet1 with a table over A1:B4."""
    import io
    buf = io.BytesIO()
    sheet = (f'<worksheet {SML}><dimension ref="A1:B4"/><sheetData>'
             '<row r="1"><c r="B1" t="s"><v>0</v></c></row>'
             '<row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2"><v>4.3</v></c></row>'
             '<row r="3"><c r="A3" t="s"><v>2</v></c><c r="B3"><v>2.5</v></c></row>'
             '<row r="4"><c r="A4" t="s"><v>3</v></c><c r="B4"><v>3.5</v></c></row>'
             '</sheetData><tableParts count="1"><tablePart r:id="rId1"/></tableParts></worksheet>')
    table = ('<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Table1" displayName="Table1" ref="A1:B4" totalsRowShown="0">'
             '<tableColumns count="2"><tableColumn id="1" name=" "/><tableColumn id="2" name="Sales"/></tableColumns>'
             '<tableStyleInfo name="TableStyleMedium2" showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/></table>')
    sst = ('<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="4">'
           '<si><t>Sales</t></si><si><t>Q1</t></si><si><t>Q2</t></si><si><t>Q3</t></si></sst>')
    rel = lambda rid, t, target: f'<Relationship Id="{rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{t}" Target="{target}"/>'
    rels = lambda *r: XL_HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + "".join(r) + '</Relationships>'
    ov = lambda part, ct: f'<Override PartName="{part}" ContentType="application/vnd.openxmlformats-officedocument.{ct}"/>'
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", XL_HDR + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>'
                   + ov("/xl/workbook.xml", "spreadsheetml.sheet.main+xml") + ov("/xl/worksheets/sheet1.xml", "spreadsheetml.worksheet+xml")
                   + ov("/xl/sharedStrings.xml", "spreadsheetml.sharedStrings+xml") + ov("/xl/tables/table1.xml", "spreadsheetml.table+xml") + "</Types>")
        z.writestr("_rels/.rels", rels(rel("rId1", "officeDocument", "xl/workbook.xml")))
        z.writestr("xl/workbook.xml", XL_HDR + f'<workbook {SML}><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>')
        z.writestr("xl/_rels/workbook.xml.rels", rels(rel("rId1", "worksheet", "worksheets/sheet1.xml"), rel("rId2", "sharedStrings", "sharedStrings.xml")))
        z.writestr("xl/worksheets/sheet1.xml", XL_HDR + sheet)
        z.writestr("xl/worksheets/_rels/sheet1.xml.rels", rels(rel("rId1", "table", "../tables/table1.xml")))
        z.writestr("xl/tables/table1.xml", XL_HDR + table)
        z.writestr("xl/sharedStrings.xml", XL_HDR + sst)
    return buf.getvalue()


def charts_pptx(path, deck):
    """deck.pptx's first slide with a column chart (and its data sheet) instead of its subtitle."""
    with zipfile.ZipFile(deck) as z:
        files = {n: z.read(n) for n in z.namelist()}
    frame = ('<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Chart 3"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>'
             '<p:xfrm><a:off x="1524000" y="3602038"/><a:ext cx="9144000" cy="3000000"/></p:xfrm><a:graphic>'
             '<a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId2"/>'
             '</a:graphicData></a:graphic></p:graphicFrame>')
    s1 = files["ppt/slides/slide1.xml"].decode()
    start = s1.find('<p:sp><p:nvSpPr><p:cNvPr id="3"')
    end = s1.find("</p:sp>", start) + len("</p:sp>")
    files["ppt/slides/slide1.xml"] = (s1[:start] + frame + s1[end:]).encode()
    r = files["ppt/slides/_rels/slide1.xml.rels"].decode()
    files["ppt/slides/_rels/slide1.xml.rels"] = r.replace("</Relationships>", '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>').encode()
    files["ppt/charts/chart1.xml"] = (XL_HDR + CHART).encode()
    files["ppt/charts/_rels/chart1.xml.rels"] = (XL_HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="../embeddings/Microsoft_Excel_Worksheet.xlsx"/></Relationships>').encode()
    files["ppt/embeddings/Microsoft_Excel_Worksheet.xlsx"] = chart_workbook()
    ct = files["[Content_Types].xml"].decode()
    files["[Content_Types].xml"] = ct.replace("</Types>", '<Default Extension="xlsx" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"/><Override PartName="/ppt/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/></Types>').encode()
    write(path, {k: v for k, v in files.items()})


if __name__ == "__main__":
    xlsx(os.path.join(out, "book.xlsx"))


P_NS = 'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"'

THEME = """<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme"><a:themeElements><a:clrScheme name="Office"><a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1><a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2><a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="ED7D31"/></a:accent2><a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4><a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6><a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink></a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Hang" typeface="맑은 고딕"/></a:majorFont><a:minorFont><a:latin typeface="Calibri"/><a:ea typeface=""/><a:cs typeface=""/><a:font script="Hang" typeface="맑은 고딕"/></a:minorFont></a:fontScheme><a:fmtScheme name="Office"><a:fillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:fillStyleLst><a:lnStyleLst><a:ln w="6350"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="12700"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln><a:ln w="19050"><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill><a:solidFill><a:schemeClr val="phClr"/></a:solidFill></a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>"""

def ph_sp(id_, name, ph, x=None, body='<a:p><a:endParaRPr lang="en-US"/></a:p>', extra_sppr=''):
    xfrm = f'<a:xfrm><a:off x="{x[0]}" y="{x[1]}"/><a:ext cx="{x[2]}" cy="{x[3]}"/></a:xfrm>' if x else ''
    return f'<p:sp><p:nvSpPr><p:cNvPr id="{id_}" name="{name}"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr>{xfrm}{extra_sppr}</p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{body}</p:txBody></p:sp>'

TREE0 = '<p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>'

def png_1x1():
    import struct, zlib
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    raw = b"".join(b"\x00" + bytes([200, 60, 60]) * 8 for _ in range(8))
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 8, 8, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


def pptx(path):
    master = f"""<p:sldMaster {P_NS}><p:cSld><p:bg><p:bgRef idx="1001"><a:schemeClr val="bg1"/></p:bgRef></p:bg><p:spTree>{TREE0}{ph_sp(2, "Title Placeholder 1", '<p:ph type="title"/>', (838200, 365125, 10515600, 1325563))}{ph_sp(3, "Text Placeholder 2", '<p:ph type="body" idx="1"/>', (838200, 1825625, 10515600, 4351338))}{ph_sp(4, "Slide Number Placeholder 5", '<p:ph type="sldNum" sz="quarter" idx="4"/>', (8610600, 6356350, 2743200, 365125), '<a:p><a:fld id="{{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}}" type="slidenum"><a:rPr lang="en-US"/><a:t>&lt;#&gt;</a:t></a:fld><a:endParaRPr lang="en-US"/></a:p>')}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/><p:sldLayoutIdLst><p:sldLayoutId id="2147483649" r:id="rId1"/><p:sldLayoutId id="2147483650" r:id="rId2"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr algn="l" defTabSz="914400" rtl="0" eaLnBrk="1" latinLnBrk="0" hangingPunct="1"><a:lnSpc><a:spcPct val="90000"/></a:lnSpc><a:spcBef><a:spcPct val="0"/></a:spcBef><a:buNone/><a:defRPr sz="4400" kern="1200"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mj-lt"/><a:ea typeface="+mj-ea"/><a:cs typeface="+mj-cs"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle><a:lvl1pPr marL="228600" indent="-228600" algn="l" defTabSz="914400"><a:lnSpc><a:spcPct val="90000"/></a:lnSpc><a:spcBef><a:spcPts val="1000"/></a:spcBef><a:buFont typeface="Arial"/><a:buChar char="•"/><a:defRPr sz="2800" kern="1200"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/><a:ea typeface="+mn-ea"/><a:cs typeface="+mn-cs"/></a:defRPr></a:lvl1pPr><a:lvl2pPr marL="685800" indent="-228600" algn="l" defTabSz="914400"><a:lnSpc><a:spcPct val="90000"/></a:lnSpc><a:spcBef><a:spcPts val="500"/></a:spcBef><a:buFont typeface="Arial"/><a:buChar char="•"/><a:defRPr sz="2400" kern="1200"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/><a:ea typeface="+mn-ea"/><a:cs typeface="+mn-cs"/></a:defRPr></a:lvl2pPr></p:bodyStyle><p:otherStyle><a:defPPr><a:defRPr lang="en-US"/></a:defPPr><a:lvl1pPr marL="0" algn="l" defTabSz="914400"><a:defRPr sz="1800" kern="1200"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/><a:ea typeface="+mn-ea"/><a:cs typeface="+mn-cs"/></a:defRPr></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"""
    layout1 = f"""<p:sldLayout {P_NS} type="title" preserve="1"><p:cSld name="Title Slide"><p:spTree>{TREE0}{ph_sp(2, "Title 1", '<p:ph type="ctrTitle"/>', (1524000, 1122363, 9144000, 2387600), '<a:p><a:r><a:rPr lang="en-US"/><a:t>Click to edit Master title style</a:t></a:r></a:p>')}{ph_sp(3, "Subtitle 2", '<p:ph type="subTitle" idx="1"/>', (1524000, 3602038, 9144000, 1655762))}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"""
    layout2 = f"""<p:sldLayout {P_NS} type="obj" preserve="1"><p:cSld name="Title and Content"><p:spTree>{TREE0}{ph_sp(2, "Title 1", '<p:ph type="title"/>')}{ph_sp(3, "Content Placeholder 2", '<p:ph idx="1"/>')}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>"""
    s1 = f"""<p:sld {P_NS}><p:cSld><p:spTree>{TREE0}{ph_sp(2, "Title 1", '<p:ph type="ctrTitle"/>', None, '<a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Quarterly </a:t></a:r><a:r><a:rPr lang="en-US" b="1" dirty="0"/><a:t>Results 2023</a:t></a:r><a:endParaRPr lang="en-US" dirty="0"/></a:p>')}{ph_sp(3, "Subtitle 2", '<p:ph type="subTitle" idx="1"/>', None, '<a:p><a:r><a:rPr lang="ko-KR" altLang="en-US" dirty="0"/><a:t>재무팀 · 2023년 4분기</a:t></a:r></a:p>')}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"""
    table = '<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="5" name="Table 4"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp="1"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x="838200" y="4300000"/><a:ext cx="6096000" cy="741680"/></p:xfrm><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/table"><a:tbl><a:tblPr firstRow="1" bandRow="1"><a:tableStyleId>{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}</a:tableStyleId></a:tblPr><a:tblGrid><a:gridCol w="3048000"/><a:gridCol w="3048000"/></a:tblGrid><a:tr h="370840"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Metric</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>2023</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr><a:tr h="370840"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Revenue</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>$1.2M</a:t></a:r></a:p></a:txBody><a:tcPr/></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>'
    group = '<p:grpSp><p:nvGrpSpPr><p:cNvPr id="6" name="Group 5"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x="7500000" y="4300000"/><a:ext cx="3000000" cy="1000000"/><a:chOff x="7500000" y="4300000"/><a:chExt cx="3000000" cy="1000000"/></a:xfrm></p:grpSpPr><p:sp><p:nvSpPr><p:cNvPr id="7" name="Rectangle 6"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="7500000" y="4300000"/><a:ext cx="3000000" cy="1000000"/></a:xfrm><a:prstGeom prst="roundRect"><a:avLst/></a:prstGeom><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></p:spPr><p:txBody><a:bodyPr rtlCol="0" anchor="ctr"/><a:lstStyle/><a:p><a:pPr algn="ctr"/><a:r><a:rPr lang="en-US" dirty="0"><a:solidFill><a:schemeClr val="bg1"/></a:solidFill></a:rPr><a:t>Up 8% in 2023</a:t></a:r></a:p></p:txBody></p:sp></p:grpSp>'
    pic = '<p:pic><p:nvPicPr><p:cNvPr id="8" name="Picture 7" descr="red square"/><p:cNvPicPr><a:picLocks noChangeAspect="1"/></p:cNvPicPr><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed="rId3"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr><a:xfrm><a:off x="10000000" y="300000"/><a:ext cx="1000000" cy="1000000"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr></p:pic>'
    s2 = f"""<p:sld {P_NS}><p:cSld><p:spTree>{TREE0}{ph_sp(2, "Title 1", '<p:ph type="title"/>', None, '<a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Highlights of 2023</a:t></a:r><a:endParaRPr lang="en-US" dirty="0"/></a:p>')}{ph_sp(3, "Content Placeholder 2", '<p:ph idx="1"/>', (838200, 1825625, 6000000, 2300000), '<a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Revenue grew </a:t></a:r><a:r><a:rPr lang="en-US" b="1" dirty="0"/><a:t>8%</a:t></a:r></a:p><a:p><a:pPr lvl="1"/><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Driven by new markets</a:t></a:r></a:p><a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Costs held flat</a:t></a:r><a:br><a:rPr lang="en-US" dirty="0"/></a:br><a:r><a:rPr lang="en-US" dirty="0"/><a:t>despite inflation</a:t></a:r></a:p>')}{table}{group}{pic}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>"""
    notes = f"""<p:notes {P_NS}><p:cSld><p:spTree>{TREE0}<p:sp><p:nvSpPr><p:cNvPr id="2" name="Slide Image Placeholder 1"/><p:cNvSpPr><a:spLocks noGrp="1" noRot="1" noChangeAspect="1"/></p:cNvSpPr><p:nvPr><p:ph type="sldImg"/></p:nvPr></p:nvSpPr><p:spPr/></p:sp>{ph_sp(3, "Notes Placeholder 2", '<p:ph type="body" idx="1"/>', None, '<a:p><a:r><a:rPr lang="en-US" dirty="0"/><a:t>Mention the 2023 market entry.</a:t></a:r></a:p>')}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:notes>"""
    notes_master = f"""<p:notesMaster {P_NS}><p:cSld><p:spTree>{TREE0}{ph_sp(2, "Notes Placeholder 1", '<p:ph type="body" sz="quarter" idx="3"/>', (685800, 4400550, 5486400, 3600450))}</p:spTree></p:cSld><p:clrMap bg1="lt1" tx1="dk1" bg2="lt2" tx2="dk2" accent1="accent1" accent2="accent2" accent3="accent3" accent4="accent4" accent5="accent5" accent6="accent6" hlink="hlink" folHlink="folHlink"/></p:notesMaster>"""
    pres = f"""<p:presentation {P_NS} saveSubsetFonts="1"><p:sldMasterIdLst><p:sldMasterId id="2147483648" r:id="rId1"/></p:sldMasterIdLst><p:notesMasterIdLst><p:notesMasterId r:id="rId4"/></p:notesMasterIdLst><p:sldIdLst><p:sldId id="256" r:id="rId2"/><p:sldId id="257" r:id="rId3"/></p:sldIdLst><p:sldSz cx="12192000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/><p:defaultTextStyle><a:lvl1pPr marL="0" algn="l" defTabSz="914400"><a:defRPr sz="1800" kern="1200"><a:solidFill><a:schemeClr val="tx1"/></a:solidFill><a:latin typeface="+mn-lt"/><a:ea typeface="+mn-ea"/><a:cs typeface="+mn-cs"/></a:defRPr></a:lvl1pPr></p:defaultTextStyle></p:presentation>"""
    rel = lambda rid, t, target: f'<Relationship Id="{rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{t}" Target="{target}"/>'
    rels = lambda *r: XL_HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + "".join(r) + '</Relationships>'
    ov = lambda part, ct: f'<Override PartName="{part}" ContentType="application/vnd.openxmlformats-officedocument.{ct}"/>'
    files = {
        "[Content_Types].xml": XL_HDR + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="png" ContentType="image/png"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>'
        + ov("/ppt/presentation.xml", "presentationml.presentation.main+xml") + ov("/ppt/slideMasters/slideMaster1.xml", "presentationml.slideMaster+xml")
        + ov("/ppt/slideLayouts/slideLayout1.xml", "presentationml.slideLayout+xml") + ov("/ppt/slideLayouts/slideLayout2.xml", "presentationml.slideLayout+xml")
        + ov("/ppt/slides/slide1.xml", "presentationml.slide+xml") + ov("/ppt/slides/slide2.xml", "presentationml.slide+xml")
        + ov("/ppt/notesSlides/notesSlide1.xml", "presentationml.notesSlide+xml") + ov("/ppt/notesMasters/notesMaster1.xml", "presentationml.notesMaster+xml")
        + ov("/ppt/theme/theme1.xml", "theme+xml") + ov("/ppt/theme/theme2.xml", "theme+xml") + '</Types>',
        "_rels/.rels": rels('<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="ppt/presentation.xml"/>'),
        "ppt/presentation.xml": XL_HDR + pres,
        "ppt/_rels/presentation.xml.rels": rels(rel("rId1", "slideMaster", "slideMasters/slideMaster1.xml"), rel("rId2", "slide", "slides/slide1.xml"), rel("rId3", "slide", "slides/slide2.xml"), rel("rId4", "notesMaster", "notesMasters/notesMaster1.xml"), rel("rId5", "theme", "theme/theme1.xml")),
        "ppt/slideMasters/slideMaster1.xml": XL_HDR + master,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels": rels(rel("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml"), rel("rId2", "slideLayout", "../slideLayouts/slideLayout2.xml"), rel("rId3", "theme", "../theme/theme1.xml")),
        "ppt/slideLayouts/slideLayout1.xml": XL_HDR + layout1,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels": rels(rel("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")),
        "ppt/slideLayouts/slideLayout2.xml": XL_HDR + layout2,
        "ppt/slideLayouts/_rels/slideLayout2.xml.rels": rels(rel("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")),
        "ppt/slides/slide1.xml": XL_HDR + s1,
        "ppt/slides/_rels/slide1.xml.rels": rels(rel("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")),
        "ppt/slides/slide2.xml": XL_HDR + s2,
        "ppt/slides/_rels/slide2.xml.rels": rels(rel("rId1", "slideLayout", "../slideLayouts/slideLayout2.xml"), rel("rId2", "notesSlide", "../notesSlides/notesSlide1.xml"), rel("rId3", "image", "../media/image1.png")),
        "ppt/notesSlides/notesSlide1.xml": XL_HDR + notes,
        "ppt/notesSlides/_rels/notesSlide1.xml.rels": rels(rel("rId1", "notesMaster", "../notesMasters/notesMaster1.xml"), rel("rId2", "slide", "../slides/slide2.xml")),
        "ppt/notesMasters/notesMaster1.xml": XL_HDR + notes_master,
        "ppt/notesMasters/_rels/notesMaster1.xml.rels": rels(rel("rId1", "theme", "../theme/theme2.xml")),
        "ppt/theme/theme1.xml": XL_HDR + THEME,
        "ppt/theme/theme2.xml": XL_HDR + THEME,
        "ppt/media/image1.png": png_1x1(),
    }
    write(path, files)


CHART = """<c:chartSpace xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><c:date1904 val="0"/><c:lang val="en-US"/><c:roundedCorners val="0"/><c:chart><c:autoTitleDeleted val="1"/><c:plotArea><c:layout/><c:barChart><c:barDir val="col"/><c:grouping val="clustered"/><c:varyColors val="0"/><c:ser><c:idx val="0"/><c:order val="0"/><c:tx><c:strRef><c:f>Sheet1!$B$1</c:f><c:strCache><c:ptCount val="1"/><c:pt idx="0"><c:v>Sales</c:v></c:pt></c:strCache></c:strRef></c:tx><c:spPr><a:solidFill><a:schemeClr val="accent1"/></a:solidFill></c:spPr><c:invertIfNegative val="0"/><c:cat><c:strRef><c:f>Sheet1!$A$2:$A$4</c:f><c:strCache><c:ptCount val="3"/><c:pt idx="0"><c:v>Q1</c:v></c:pt><c:pt idx="1"><c:v>Q2</c:v></c:pt><c:pt idx="2"><c:v>Q3</c:v></c:pt></c:strCache></c:strRef></c:cat><c:val><c:numRef><c:f>Sheet1!$B$2:$B$4</c:f><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val="3"/><c:pt idx="0"><c:v>4.3</c:v></c:pt><c:pt idx="1"><c:v>2.5</c:v></c:pt><c:pt idx="2"><c:v>3.5</c:v></c:pt></c:numCache></c:numRef></c:val></c:ser><c:gapWidth val="219"/><c:overlap val="-27"/><c:axId val="1"/><c:axId val="2"/></c:barChart><c:catAx><c:axId val="1"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="b"/><c:numFmt formatCode="General" sourceLinked="1"/><c:majorTickMark val="none"/><c:minorTickMark val="none"/><c:tickLblPos val="nextTo"/><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="1197"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="65000"/><a:lumOff val="35000"/></a:schemeClr></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr><c:crossAx val="2"/><c:crosses val="autoZero"/><c:auto val="1"/><c:lblAlgn val="ctr"/><c:lblOffset val="100"/><c:noMultiLvlLbl val="0"/></c:catAx><c:valAx><c:axId val="2"/><c:scaling><c:orientation val="minMax"/></c:scaling><c:delete val="0"/><c:axPos val="l"/><c:majorGridlines><c:spPr><a:ln w="9525"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="15000"/><a:lumOff val="85000"/></a:schemeClr></a:solidFill></a:ln></c:spPr></c:majorGridlines><c:numFmt formatCode="General" sourceLinked="1"/><c:majorTickMark val="none"/><c:minorTickMark val="none"/><c:tickLblPos val="nextTo"/><c:spPr><a:ln><a:noFill/></a:ln></c:spPr><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr sz="1197"><a:solidFill><a:schemeClr val="tx1"><a:lumMod val="65000"/><a:lumOff val="35000"/></a:schemeClr></a:solidFill></a:defRPr></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr><c:crossAx val="1"/><c:crosses val="autoZero"/><c:crossBetween val="between"/></c:valAx></c:plotArea><c:legend><c:legendPos val="b"/><c:overlay val="0"/></c:legend><c:plotVisOnly val="1"/><c:dispBlanksAs val="gap"/></c:chart><c:txPr><a:bodyPr/><a:lstStyle/><a:p><a:pPr><a:defRPr/></a:pPr><a:endParaRPr lang="en-US"/></a:p></c:txPr><c:externalData r:id="rId1"><c:autoUpdate val="0"/></c:externalData></c:chartSpace>"""


def chart_workbook():
    """The chart's data sheet, as PowerPoint embeds it: Sheet1 with a table over A1:B4."""
    import io
    buf = io.BytesIO()
    sheet = (f'<worksheet {SML}><dimension ref="A1:B4"/><sheetData>'
             '<row r="1"><c r="B1" t="s"><v>0</v></c></row>'
             '<row r="2"><c r="A2" t="s"><v>1</v></c><c r="B2"><v>4.3</v></c></row>'
             '<row r="3"><c r="A3" t="s"><v>2</v></c><c r="B3"><v>2.5</v></c></row>'
             '<row r="4"><c r="A4" t="s"><v>3</v></c><c r="B4"><v>3.5</v></c></row>'
             '</sheetData><tableParts count="1"><tablePart r:id="rId1"/></tableParts></worksheet>')
    table = ('<table xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" id="1" name="Table1" displayName="Table1" ref="A1:B4" totalsRowShown="0">'
             '<tableColumns count="2"><tableColumn id="1" name=" "/><tableColumn id="2" name="Sales"/></tableColumns>'
             '<tableStyleInfo name="TableStyleMedium2" showFirstColumn="0" showLastColumn="0" showRowStripes="1" showColumnStripes="0"/></table>')
    sst = ('<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="4">'
           '<si><t>Sales</t></si><si><t>Q1</t></si><si><t>Q2</t></si><si><t>Q3</t></si></sst>')
    rel = lambda rid, t, target: f'<Relationship Id="{rid}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{t}" Target="{target}"/>'
    rels = lambda *r: XL_HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">' + "".join(r) + '</Relationships>'
    ov = lambda part, ct: f'<Override PartName="{part}" ContentType="application/vnd.openxmlformats-officedocument.{ct}"/>'
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", XL_HDR + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>'
                   + ov("/xl/workbook.xml", "spreadsheetml.sheet.main+xml") + ov("/xl/worksheets/sheet1.xml", "spreadsheetml.worksheet+xml")
                   + ov("/xl/sharedStrings.xml", "spreadsheetml.sharedStrings+xml") + ov("/xl/tables/table1.xml", "spreadsheetml.table+xml") + "</Types>")
        z.writestr("_rels/.rels", rels(rel("rId1", "officeDocument", "xl/workbook.xml")))
        z.writestr("xl/workbook.xml", XL_HDR + f'<workbook {SML}><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>')
        z.writestr("xl/_rels/workbook.xml.rels", rels(rel("rId1", "worksheet", "worksheets/sheet1.xml"), rel("rId2", "sharedStrings", "sharedStrings.xml")))
        z.writestr("xl/worksheets/sheet1.xml", XL_HDR + sheet)
        z.writestr("xl/worksheets/_rels/sheet1.xml.rels", rels(rel("rId1", "table", "../tables/table1.xml")))
        z.writestr("xl/tables/table1.xml", XL_HDR + table)
        z.writestr("xl/sharedStrings.xml", XL_HDR + sst)
    return buf.getvalue()


def charts_pptx(path, deck):
    """deck.pptx's first slide with a column chart (and its data sheet) instead of its subtitle."""
    with zipfile.ZipFile(deck) as z:
        files = {n: z.read(n) for n in z.namelist()}
    frame = ('<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="4" name="Chart 3"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr>'
             '<p:xfrm><a:off x="1524000" y="3602038"/><a:ext cx="9144000" cy="3000000"/></p:xfrm><a:graphic>'
             '<a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/chart"><c:chart xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" r:id="rId2"/>'
             '</a:graphicData></a:graphic></p:graphicFrame>')
    s1 = files["ppt/slides/slide1.xml"].decode()
    start = s1.find('<p:sp><p:nvSpPr><p:cNvPr id="3"')
    end = s1.find("</p:sp>", start) + len("</p:sp>")
    files["ppt/slides/slide1.xml"] = (s1[:start] + frame + s1[end:]).encode()
    r = files["ppt/slides/_rels/slide1.xml.rels"].decode()
    files["ppt/slides/_rels/slide1.xml.rels"] = r.replace("</Relationships>", '<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/></Relationships>').encode()
    files["ppt/charts/chart1.xml"] = (XL_HDR + CHART).encode()
    files["ppt/charts/_rels/chart1.xml.rels"] = (XL_HDR + '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/package" Target="../embeddings/Microsoft_Excel_Worksheet.xlsx"/></Relationships>').encode()
    files["ppt/embeddings/Microsoft_Excel_Worksheet.xlsx"] = chart_workbook()
    ct = files["[Content_Types].xml"].decode()
    files["[Content_Types].xml"] = ct.replace("</Types>", '<Default Extension="xlsx" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"/><Override PartName="/ppt/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/></Types>').encode()
    write(path, {k: v for k, v in files.items()})


if __name__ == "__main__":
    pptx(os.path.join(out, "deck.pptx"))
    charts_pptx(os.path.join(out, "charts.pptx"), os.path.join(out, "deck.pptx"))

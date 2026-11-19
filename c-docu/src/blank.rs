//! New, empty documents: a Word document with the usual styles and lists, a workbook with one
//! sheet, a presentation with a theme, a master, the common layouts and a title slide — each as
//! Office itself would write it, so that later edits find what they expect.

use crate::zip::Zip;
use crate::{err, Result};

const H: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

fn rels(items: &[(&str, &str, &str)]) -> Vec<u8> {
    let mut s = format!("{H}<Relationships xmlns=\"{PKG_REL}\">");
    for (id, typ, target) in items {
        s.push_str(&format!("<Relationship Id=\"{id}\" Type=\"{REL}/{typ}\" Target=\"{target}\"/>"));
    }
    s.push_str("</Relationships>");
    s.into_bytes()
}

fn types(defaults: &[(&str, &str)], overrides: &[(&str, &str)]) -> Vec<u8> {
    let mut s = format!("{H}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>");
    for (e, t) in defaults {
        s.push_str(&format!("<Default Extension=\"{e}\" ContentType=\"{t}\"/>"));
    }
    for (p, t) in overrides {
        s.push_str(&format!("<Override PartName=\"{p}\" ContentType=\"application/vnd.openxmlformats-officedocument.{t}\"/>"));
    }
    s.push_str("</Types>");
    s.into_bytes()
}

/// A new document of the kind the file name's extension names.
pub fn new(path: &str) -> Result<Vec<u8>> {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "docx" => docx(),
        "xlsx" => xlsx(),
        "pptx" => pptx(),
        _ => Err(err(format!("new makes .docx, .xlsx or .pptx files, not .{ext}"))),
    }
}

const W_NS: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

fn docx() -> Result<Vec<u8>> {
    let mut z = Zip::new();
    z.write(
        "[Content_Types].xml",
        types(
            &[],
            &[
                ("/word/document.xml", "wordprocessingml.document.main+xml"),
                ("/word/styles.xml", "wordprocessingml.styles+xml"),
                ("/word/settings.xml", "wordprocessingml.settings+xml"),
                ("/word/numbering.xml", "wordprocessingml.numbering+xml"),
            ],
        ),
    );
    z.write("_rels/.rels", rels(&[("rId1", "officeDocument", "word/document.xml")]));
    z.write("word/_rels/document.xml.rels", rels(&[("rId1", "styles", "styles.xml"), ("rId2", "settings", "settings.xml"), ("rId3", "numbering", "numbering.xml")]));
    z.write(
        "word/document.xml",
        format!("{H}<w:document {W_NS}><w:body><w:p/><w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"851\" w:footer=\"992\" w:gutter=\"0\"/><w:cols w:space=\"425\"/><w:docGrid w:linePitch=\"360\"/></w:sectPr></w:body></w:document>").into_bytes(),
    );
    z.write("word/settings.xml", format!("{H}<w:settings {W_NS}><w:defaultTabStop w:val=\"720\"/><w:characterSpacingControl w:val=\"doNotCompress\"/><w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat></w:settings>").into_bytes());
    let heading = |id: &str, name: &str, lvl: u8, size: u32, color: &str, before: u32| {
        format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"{id}\"><w:name w:val=\"{name}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before=\"{before}\" w:after=\"80\"/><w:outlineLvl w:val=\"{lvl}\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Calibri Light\" w:hAnsi=\"Calibri Light\"/><w:color w:val=\"{color}\"/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr></w:style>"
        )
    };
    let styles = format!(
        "{H}<w:styles {W_NS}><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:eastAsia=\"Malgun Gothic\" w:hAnsi=\"Calibri\" w:cs=\"Times New Roman\"/><w:kern w:val=\"2\"/><w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/><w:lang w:val=\"en-US\" w:eastAsia=\"ko-KR\" w:bidi=\"ar-SA\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after=\"160\" w:line=\"259\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>\
<w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\"><w:name w:val=\"Default Paragraph Font\"/><w:uiPriority w:val=\"1\"/><w:semiHidden/></w:style>\
<w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\"><w:name w:val=\"Normal Table\"/><w:semiHidden/><w:tblPr><w:tblInd w:w=\"0\" w:type=\"dxa\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"108\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>\
<w:style w:type=\"numbering\" w:default=\"1\" w:styleId=\"NoList\"><w:name w:val=\"No List\"/><w:semiHidden/></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:spacing w:after=\"80\" w:line=\"240\" w:lineRule=\"auto\"/><w:contextualSpacing/></w:pPr><w:rPr><w:rFonts w:ascii=\"Calibri Light\" w:hAnsi=\"Calibri Light\"/><w:spacing w:val=\"-10\"/><w:kern w:val=\"28\"/><w:sz w:val=\"56\"/><w:szCs w:val=\"56\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Subtitle\"><w:name w:val=\"Subtitle\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:rPr><w:color w:val=\"595959\"/><w:spacing w:val=\"15\"/><w:sz w:val=\"28\"/><w:szCs w:val=\"28\"/></w:rPr></w:style>\
{}{}{}{}\
<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:uiPriority w:val=\"34\"/><w:qFormat/><w:pPr><w:ind w:left=\"720\"/><w:contextualSpacing/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:spacing w:before=\"160\"/><w:jc w:val=\"center\"/></w:pPr><w:rPr><w:i/><w:color w:val=\"404040\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Caption\"><w:name w:val=\"caption\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/><w:pPr><w:spacing w:after=\"200\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:rPr><w:i/><w:color w:val=\"44546A\"/><w:sz w:val=\"18\"/><w:szCs w:val=\"18\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Header\"><w:name w:val=\"header\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:tabs><w:tab w:val=\"center\" w:pos=\"4513\"/><w:tab w:val=\"right\" w:pos=\"9026\"/></w:tabs><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Footer\"><w:name w:val=\"footer\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:tabs><w:tab w:val=\"center\" w:pos=\"4513\"/><w:tab w:val=\"right\" w:pos=\"9026\"/></w:tabs><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"FootnoteText\"><w:name w:val=\"footnote text\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:rPr><w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/></w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"FootnoteReference\"><w:name w:val=\"footnote reference\"/><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"Strong\"><w:name w:val=\"Strong\"/><w:qFormat/><w:rPr><w:b/><w:bCs/></w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"Emphasis\"><w:name w:val=\"Emphasis\"/><w:qFormat/><w:rPr><w:i/><w:iCs/></w:rPr></w:style>\
<w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:basedOn w:val=\"TableNormal\"/><w:pPr><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:tblPr><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"auto\"/></w:tblBorders></w:tblPr></w:style>\
</w:styles>",
        heading("Heading1", "heading 1", 0, 32, "2F5496", 360),
        heading("Heading2", "heading 2", 1, 26, "2F5496", 160),
        heading("Heading3", "heading 3", 2, 24, "1F3763", 160),
        heading("Heading4", "heading 4", 3, 22, "2F5496", 80),
    );
    z.write("word/styles.xml", styles.into_bytes());
    // Two lists: bullets (numId 1) and numbers (numId 2).
    let mut num = format!("{H}<w:numbering {W_NS}>");
    for (id, bullet) in [(0, true), (1, false)] {
        num.push_str(&format!("<w:abstractNum w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>"));
        for l in 0..9u32 {
            let (fmt, text, font) = if bullet {
                let c = ["•", "◦", "▪"][l as usize % 3];
                ("bullet", c.to_string(), "<w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:hint=\"default\"/></w:rPr>")
            } else {
                let f = ["decimal", "lowerLetter", "lowerRoman"][l as usize % 3];
                (f, format!("%{}.", l + 1), "")
            };
            num.push_str(&format!(
                "<w:lvl w:ilvl=\"{l}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"{text}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{}\" w:hanging=\"360\"/></w:pPr>{font}</w:lvl>",
                720 + 720 * l
            ));
        }
        num.push_str("</w:abstractNum>");
    }
    num.push_str("<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num><w:num w:numId=\"2\"><w:abstractNumId w:val=\"1\"/></w:num></w:numbering>");
    z.write("word/numbering.xml", num.into_bytes());
    z.save()
}

const S_NS: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

fn xlsx() -> Result<Vec<u8>> {
    let mut z = Zip::new();
    z.write(
        "[Content_Types].xml",
        types(
            &[],
            &[
                ("/xl/workbook.xml", "spreadsheetml.sheet.main+xml"),
                ("/xl/worksheets/sheet1.xml", "spreadsheetml.worksheet+xml"),
                ("/xl/styles.xml", "spreadsheetml.styles+xml"),
                ("/xl/theme/theme1.xml", "theme+xml"),
            ],
        ),
    );
    z.write("_rels/.rels", rels(&[("rId1", "officeDocument", "xl/workbook.xml")]));
    z.write("xl/_rels/workbook.xml.rels", rels(&[("rId1", "worksheet", "worksheets/sheet1.xml"), ("rId2", "styles", "styles.xml"), ("rId3", "theme", "theme/theme1.xml")]));
    z.write(
        "xl/workbook.xml",
        format!("{H}<workbook {S_NS}><workbookPr defaultThemeVersion=\"166925\"/><bookViews><workbookView xWindow=\"0\" yWindow=\"0\" windowWidth=\"28800\" windowHeight=\"12300\"/></bookViews><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets><calcPr calcId=\"191029\"/></workbook>").into_bytes(),
    );
    z.write(
        "xl/worksheets/sheet1.xml",
        format!("{H}<worksheet {S_NS}><dimension ref=\"A1\"/><sheetViews><sheetView tabSelected=\"1\" workbookViewId=\"0\"/></sheetViews><sheetFormatPr defaultRowHeight=\"15\"/><sheetData/><pageMargins left=\"0.7\" right=\"0.7\" top=\"0.75\" bottom=\"0.75\" header=\"0.3\" footer=\"0.3\"/></worksheet>").into_bytes(),
    );
    z.write(
        "xl/styles.xml",
        format!("{H}<styleSheet {S_NS}><fonts count=\"1\"><font><sz val=\"11\"/><color theme=\"1\"/><name val=\"Calibri\"/><family val=\"2\"/><scheme val=\"minor\"/></font></fonts><fills count=\"2\"><fill><patternFill patternType=\"none\"/></fill><fill><patternFill patternType=\"gray125\"/></fill></fills><borders count=\"1\"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs><cellXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/></cellXfs><cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles><dxfs count=\"0\"/><tableStyles count=\"0\" defaultTableStyle=\"TableStyleMedium2\" defaultPivotStyle=\"PivotStyleLight16\"/></styleSheet>").into_bytes(),
    );
    z.write("xl/theme/theme1.xml", theme().into_bytes());
    z.save()
}

/// The Office theme: its colors, Calibri Light / Calibri (Malgun Gothic for Korean), and the
/// formatting schemes Office requires.
fn theme() -> String {
    let fonts = |latin: &str| format!("<a:latin typeface=\"{latin}\" panose=\"020F0302020204030204\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/><a:font script=\"Hang\" typeface=\"맑은 고딕\"/><a:font script=\"Jpan\" typeface=\"游ゴシック\"/><a:font script=\"Hans\" typeface=\"等线\"/>");
    let solid = |inner: &str| format!("<a:solidFill><a:schemeClr val=\"phClr\">{inner}</a:schemeClr></a:solidFill>");
    format!(
        "{H}<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Office Theme\"><a:themeElements><a:clrScheme name=\"Office\"><a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1><a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1><a:dk2><a:srgbClr val=\"44546A\"/></a:dk2><a:lt2><a:srgbClr val=\"E7E6E6\"/></a:lt2><a:accent1><a:srgbClr val=\"4472C4\"/></a:accent1><a:accent2><a:srgbClr val=\"ED7D31\"/></a:accent2><a:accent3><a:srgbClr val=\"A5A5A5\"/></a:accent3><a:accent4><a:srgbClr val=\"FFC000\"/></a:accent4><a:accent5><a:srgbClr val=\"5B9BD5\"/></a:accent5><a:accent6><a:srgbClr val=\"70AD47\"/></a:accent6><a:hlink><a:srgbClr val=\"0563C1\"/></a:hlink><a:folHlink><a:srgbClr val=\"954F72\"/></a:folHlink></a:clrScheme>\
<a:fontScheme name=\"Office\"><a:majorFont>{}</a:majorFont><a:minorFont>{}</a:minorFont></a:fontScheme>\
<a:fmtScheme name=\"Office\"><a:fillStyleLst>{}{}{}</a:fillStyleLst><a:lnStyleLst><a:ln w=\"6350\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\">{}<a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln><a:ln w=\"12700\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\">{}<a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln><a:ln w=\"19050\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\">{}<a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln></a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst><a:outerShdw blurRad=\"57150\" dist=\"19050\" dir=\"5400000\" algn=\"ctr\" rotWithShape=\"0\"><a:srgbClr val=\"000000\"><a:alpha val=\"63000\"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst>{}{}{}</a:bgFillStyleLst></a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>",
        fonts("Calibri Light"),
        fonts("Calibri"),
        solid(""),
        solid("<a:tint val=\"67000\"/>"),
        solid("<a:shade val=\"95000\"/>"),
        solid(""),
        solid(""),
        solid(""),
        solid(""),
        solid("<a:tint val=\"95000\"/>"),
        solid("<a:shade val=\"80000\"/>"),
    )
}

const P_NS: &str = "xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\"";

fn ph(id: u32, name: &str, ph: &str, xywh: Option<(i64, i64, i64, i64)>, body: &str) -> String {
    let xfrm = xywh.map(|(x, y, w, h)| format!("<a:xfrm><a:off x=\"{x}\" y=\"{y}\"/><a:ext cx=\"{w}\" cy=\"{h}\"/></a:xfrm>")).unwrap_or_default();
    format!(
        "<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"{name}\"/><p:cNvSpPr><a:spLocks noGrp=\"1\"/></p:cNvSpPr><p:nvPr>{ph}</p:nvPr></p:nvSpPr><p:spPr>{xfrm}</p:spPr><p:txBody><a:bodyPr/><a:lstStyle/>{body}</p:txBody></p:sp>"
    )
}

const TREE: &str = "<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";
const EMPTY_P: &str = "<a:p><a:endParaRPr lang=\"en-US\"/></a:p>";

fn pptx() -> Result<Vec<u8>> {
    let mut z = Zip::new();
    let layouts: [(&str, &str, String); 6] = [
        (
            "title",
            "Title Slide",
            ph(2, "Title 1", "<p:ph type=\"ctrTitle\"/>", Some((1524000, 1122363, 9144000, 2387600)), EMPTY_P).replace("<a:bodyPr/>", "<a:bodyPr anchor=\"b\"/>").replace("<a:lstStyle/>", "<a:lstStyle><a:lvl1pPr algn=\"ctr\"><a:defRPr sz=\"6000\"/></a:lvl1pPr></a:lstStyle>")
                + &ph(3, "Subtitle 2", "<p:ph type=\"subTitle\" idx=\"1\"/>", Some((1524000, 3602038, 9144000, 1655762)), EMPTY_P).replace("<a:lstStyle/>", "<a:lstStyle><a:lvl1pPr marL=\"0\" indent=\"0\" algn=\"ctr\"><a:buNone/><a:defRPr sz=\"2400\"/></a:lvl1pPr></a:lstStyle>"),
        ),
        ("obj", "Title and Content", ph(2, "Title 1", "<p:ph type=\"title\"/>", None, EMPTY_P) + &ph(3, "Content Placeholder 2", "<p:ph idx=\"1\"/>", None, EMPTY_P)),
        (
            "secHead",
            "Section Header",
            ph(2, "Title 1", "<p:ph type=\"title\"/>", Some((831850, 1709738, 10515600, 2852737)), EMPTY_P).replace("<a:bodyPr/>", "<a:bodyPr anchor=\"b\"/>").replace("<a:lstStyle/>", "<a:lstStyle><a:lvl1pPr><a:defRPr sz=\"6000\"/></a:lvl1pPr></a:lstStyle>")
                + &ph(3, "Text Placeholder 2", "<p:ph type=\"body\" idx=\"1\"/>", Some((831850, 4589463, 10515600, 1500187)), EMPTY_P).replace("<a:lstStyle/>", "<a:lstStyle><a:lvl1pPr marL=\"0\" indent=\"0\"><a:buNone/><a:defRPr sz=\"2400\"><a:solidFill><a:schemeClr val=\"tx1\"><a:tint val=\"75000\"/></a:schemeClr></a:solidFill></a:defRPr></a:lvl1pPr></a:lstStyle>"),
        ),
        (
            "twoObj",
            "Two Content",
            ph(2, "Title 1", "<p:ph type=\"title\"/>", None, EMPTY_P)
                + &ph(3, "Content Placeholder 2", "<p:ph sz=\"half\" idx=\"1\"/>", Some((838200, 1825625, 5181600, 4351338)), EMPTY_P)
                + &ph(4, "Content Placeholder 3", "<p:ph sz=\"half\" idx=\"2\"/>", Some((6172200, 1825625, 5181600, 4351338)), EMPTY_P),
        ),
        ("titleOnly", "Title Only", ph(2, "Title 1", "<p:ph type=\"title\"/>", None, EMPTY_P)),
        ("blank", "Blank", String::new()),
    ];
    let mut overrides: Vec<(String, &str)> = vec![
        ("/ppt/presentation.xml".into(), "presentationml.presentation.main+xml"),
        ("/ppt/slideMasters/slideMaster1.xml".into(), "presentationml.slideMaster+xml"),
        ("/ppt/theme/theme1.xml".into(), "theme+xml"),
        ("/ppt/slides/slide1.xml".into(), "presentationml.slide+xml"),
    ];
    let mut layout_ids = String::new();
    let mut master_rels: Vec<(String, &str, String)> = Vec::new();
    for (i, (typ, name, shapes)) in layouts.iter().enumerate() {
        let n = i + 1;
        z.write(
            &format!("ppt/slideLayouts/slideLayout{n}.xml"),
            format!("{H}<p:sldLayout {P_NS} type=\"{typ}\" preserve=\"1\"><p:cSld name=\"{name}\"><p:spTree>{TREE}{shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>").into_bytes(),
        );
        z.write(&format!("ppt/slideLayouts/_rels/slideLayout{n}.xml.rels"), rels(&[("rId1", "slideMaster", "../slideMasters/slideMaster1.xml")]));
        overrides.push((format!("/ppt/slideLayouts/slideLayout{n}.xml"), "presentationml.slideLayout+xml"));
        layout_ids.push_str(&format!("<p:sldLayoutId id=\"{}\" r:id=\"rId{n}\"/>", 2147483649u64 + i as u64));
        master_rels.push((format!("rId{n}"), "slideLayout", format!("../slideLayouts/slideLayout{n}.xml")));
    }
    let theme_rid = format!("rId{}", layouts.len() + 1);
    master_rels.push((theme_rid.clone(), "theme", "../theme/theme1.xml".into()));
    let lvl = |n: u32, mar: i64, sz: u32| {
        format!(
            "<a:lvl{n}pPr marL=\"{mar}\" indent=\"-228600\" algn=\"l\" defTabSz=\"914400\" rtl=\"0\" eaLnBrk=\"1\" latinLnBrk=\"0\" hangingPunct=\"1\"><a:lnSpc><a:spcPct val=\"90000\"/></a:lnSpc><a:spcBef><a:spcPts val=\"{}\"/></a:spcBef><a:buFont typeface=\"Arial\" panose=\"020B0604020202020204\"/><a:buChar char=\"•\"/><a:defRPr sz=\"{sz}\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/><a:cs typeface=\"+mn-cs\"/></a:defRPr></a:lvl{n}pPr>",
            if n == 1 { 1000 } else { 500 }
        )
    };
    let body_style: String = [(1, 228600, 2800), (2, 685800, 2400), (3, 1143000, 2000), (4, 1600200, 1800), (5, 2057400, 1800), (6, 2514600, 1800), (7, 2971800, 1800), (8, 3429000, 1800), (9, 3886200, 1800)].iter().map(|(n, m, s)| lvl(*n, *m, *s)).collect();
    let master_shapes = ph(2, "Title Placeholder 1", "<p:ph type=\"title\"/>", Some((838200, 365125, 10515600, 1325563)), EMPTY_P).replace("<a:bodyPr/>", "<a:bodyPr vert=\"horz\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" rtlCol=\"0\" anchor=\"ctr\"><a:normAutofit/></a:bodyPr>")
        + &ph(3, "Text Placeholder 2", "<p:ph type=\"body\" idx=\"1\"/>", Some((838200, 1825625, 10515600, 4351338)), EMPTY_P).replace("<a:bodyPr/>", "<a:bodyPr vert=\"horz\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" rtlCol=\"0\"><a:normAutofit/></a:bodyPr>");
    z.write(
        "ppt/slideMasters/slideMaster1.xml",
        format!(
            "{H}<p:sldMaster {P_NS}><p:cSld><p:bg><p:bgRef idx=\"1001\"><a:schemeClr val=\"bg1\"/></p:bgRef></p:bg><p:spTree>{TREE}{master_shapes}</p:spTree></p:cSld><p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/><p:sldLayoutIdLst>{layout_ids}</p:sldLayoutIdLst><p:txStyles><p:titleStyle><a:lvl1pPr algn=\"l\" defTabSz=\"914400\" rtl=\"0\" eaLnBrk=\"1\" latinLnBrk=\"0\" hangingPunct=\"1\"><a:lnSpc><a:spcPct val=\"90000\"/></a:lnSpc><a:spcBef><a:spcPct val=\"0\"/></a:spcBef><a:buNone/><a:defRPr sz=\"4400\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mj-lt\"/><a:ea typeface=\"+mj-ea\"/><a:cs typeface=\"+mj-cs\"/></a:defRPr></a:lvl1pPr></p:titleStyle><p:bodyStyle>{body_style}</p:bodyStyle><p:otherStyle><a:defPPr><a:defRPr lang=\"en-US\"/></a:defPPr><a:lvl1pPr marL=\"0\" algn=\"l\" defTabSz=\"914400\"><a:defRPr sz=\"1800\" kern=\"1200\"><a:solidFill><a:schemeClr val=\"tx1\"/></a:solidFill><a:latin typeface=\"+mn-lt\"/><a:ea typeface=\"+mn-ea\"/><a:cs typeface=\"+mn-cs\"/></a:defRPr></a:lvl1pPr></p:otherStyle></p:txStyles></p:sldMaster>"
        )
        .into_bytes(),
    );
    let mr: Vec<(&str, &str, &str)> = master_rels.iter().map(|(a, b, c)| (a.as_str(), *b, c.as_str())).collect();
    z.write("ppt/slideMasters/_rels/slideMaster1.xml.rels", rels(&mr));
    z.write("ppt/theme/theme1.xml", theme().into_bytes());
    // One title slide.
    let slide_shapes = ph(2, "Title 1", "<p:ph type=\"ctrTitle\"/>", None, EMPTY_P) + &ph(3, "Subtitle 2", "<p:ph type=\"subTitle\" idx=\"1\"/>", None, EMPTY_P);
    z.write(
        "ppt/slides/slide1.xml",
        format!("{H}<p:sld {P_NS}><p:cSld><p:spTree>{TREE}{slide_shapes}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>").into_bytes(),
    );
    z.write("ppt/slides/_rels/slide1.xml.rels", rels(&[("rId1", "slideLayout", "../slideLayouts/slideLayout1.xml")]));
    z.write(
        "ppt/presentation.xml",
        format!("{H}<p:presentation {P_NS} saveSubsetFonts=\"1\"><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst><p:sldId id=\"256\" r:id=\"rId2\"/></p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/><p:defaultTextStyle><a:defPPr><a:defRPr lang=\"en-US\"/></a:defPPr></p:defaultTextStyle></p:presentation>").into_bytes(),
    );
    z.write("ppt/_rels/presentation.xml.rels", rels(&[("rId1", "slideMaster", "slideMasters/slideMaster1.xml"), ("rId2", "slide", "slides/slide1.xml"), ("rId3", "theme", "theme/theme1.xml")]));
    z.write("_rels/.rels", rels(&[("rId1", "officeDocument", "ppt/presentation.xml")]));
    let ov: Vec<(&str, &str)> = overrides.iter().map(|(a, b)| (a.as_str(), *b)).collect();
    z.write("[Content_Types].xml", types(&[], &ov));
    z.save()
}

"""lxml and the libraries in it (libxml2, libxslt/libexslt, GNU libiconv, zlib): parsing and
serializing XML and HTML in many encodings, the ElementTree API, XPath with Python extension
functions and EXSLT, XSLT with parameters and messages, DTD, XML Schema, RelaxNG and Schematron
validation with their error messages, objectify, the E-factory, SAX, iterparse and the feed and
pull parsers, xmlfile, gzip'ed documents, XInclude, resolvers, C14N, htmldiff; the errors lxml
raises. The documents are made here, in a temporary folder (only names relative to it printed).
tests/reference.py runs it with lxml's own wheel.
"""
import codecs
import gzip
import hashlib
import io
import os
import tempfile

from lxml import etree, objectify, sax
from lxml.builder import E, ElementMaker
from lxml.html import builder as H
import lxml.html
import lxml.html.diff

os.chdir(tempfile.mkdtemp())


def md5(b):
    return hashlib.md5(b).hexdigest()


def errors(log):
    return [(e.line, e.column, e.domain_name, e.type_name, e.level_name, e.message) for e in log]


print("versions", etree.LXML_VERSION, etree.LIBXML_VERSION, etree.LIBXML_COMPILED_VERSION, etree.LIBXSLT_VERSION,
      etree.LIBXSLT_COMPILED_VERSION)
print("features", sorted(etree.LIBXML_FEATURES))

# ── parsing and serializing ─────────────────────────────────────────────────────────────────────
DOC = b"""<?xml version="1.0"?>
<!DOCTYPE root [
  <!ENTITY greeting "hello &amp; welcome">
  <!ATTLIST item kind CDATA "plain">
]>
<?app instruction?>
<root xmlns="urn:a" xmlns:b="urn:b" b:attr="1">
  <!-- a comment -->
  <item id="1">&greeting;</item>
  <item id="2" kind="special"><![CDATA[<raw> & text]]></item>
  <b:item id="3">tail<sub/>more</b:item>
  <empty/>
</root>"""
root = etree.fromstring(DOC)
print("tree", root.tag, root.nsmap, [c.tag if isinstance(c.tag, str) else type(c).__name__ for c in root])
print("text", [(c.get("id"), c.get("kind"), c.text) for c in root.iter("{urn:a}item")])
print("sourceline", [(c.tag, c.sourceline) for c in root.iter() if isinstance(c.tag, str)])
print("serialize", etree.tostring(root))
print("pretty", etree.tostring(root, pretty_print=True).decode())
tree = root.getroottree()
print("docinfo", tree.docinfo.xml_version, tree.docinfo.encoding, tree.docinfo.root_name, tree.docinfo.doctype,
      tree.docinfo.internalDTD is not None)
print("full", tree.getroot().getprevious().text if tree.getroot().getprevious() is not None else None)
for enc in ("utf-8", "iso-8859-1", "ascii", "utf-16", "utf-32", "cp1252"):
    data = etree.tostring(root, encoding=enc, xml_declaration=True)
    print("encode", enc, len(data), md5(data))
print("method", etree.tostring(root, method="text"), etree.tostring(root, method="c14n")[:60])
print("unicode", etree.tostring(root, encoding="unicode")[:40])

# character encodings, converted by libiconv: documents written in each, parsed back
SAMPLE = "Grüße · Привет · こんにちは · 안녕하세요 · 你好 · Ωμέγα"
for enc, text in [("ISO-8859-1", "Grüße, déjà vu"), ("ISO-8859-15", "price: 5€"), ("windows-1252", "“quotes” – dash"),
                  ("KOI8-R", "Привет мир"), ("windows-1251", "Привет"), ("ISO-8859-7", "Ωμέγα"),
                  ("Shift_JIS", "こんにちは世界"), ("EUC-JP", "こんにちは"), ("ISO-2022-JP", "日本語"),
                  ("EUC-KR", "안녕하세요"), ("GB2312", "你好世界"), ("GB18030", "你好𠀀"), ("Big5", "繁體中文"),
                  ("UTF-16LE", SAMPLE), ("UTF-16BE", SAMPLE), ("UTF-32", SAMPLE), ("UTF-7", "plus+sign ü"),
                  ("TIS-620", "สวัสดี"), ("CP437", "box ─┼─"), ("MacRoman", "café ƒ")]:
    try:
        raw = f'<?xml version="1.0" encoding="{enc}"?><t>{text}</t>'.encode(codecs.lookup(enc.replace("MacRoman", "mac-roman")).name)
    except LookupError:
        print("codec", enc, "none in Python")
        continue
    try:
        t = etree.fromstring(raw)
        back = etree.tostring(t, encoding=enc)
        print("charset", enc, t.text == text, md5(back))
    except (etree.XMLSyntaxError, LookupError, ValueError) as e:
        print("charset", enc, type(e).__name__, str(e)[:80])
print("bom", etree.fromstring(codecs.BOM_UTF16_LE + "<a>é</a>".encode("utf-16-le")).text)
try:
    etree.fromstring('<?xml version="1.0" encoding="no-such-encoding"?><a/>'.encode())
except Exception as e:
    print("bad encoding", type(e).__name__, e)

# errors and recovery
for bad in [b"<a><b></a>", b"<a>&undefined;</a>", b"<a attr=1/>", b"", b"<a>\x00</a>", b"<?xml?><a/>"]:
    try:
        etree.fromstring(bad)
        print("parsed?", bad)
    except etree.XMLSyntaxError as e:
        print("syntax", bad, e.msg, e.position, e.code)
print("recover", etree.tostring(etree.fromstring(b"<a><b>text</a>", etree.XMLParser(recover=True))))
p = etree.XMLParser(remove_blank_text=True, remove_comments=True, remove_pis=True, strip_cdata=False, resolve_entities=False)
print("options", etree.tostring(etree.fromstring(DOC, p)))
print("huge", len(etree.fromstring(b"<a>" + b"<b>x</b>" * 20000 + b"</a>", etree.XMLParser(huge_tree=True))))
deep = b"<a>" * 300 + b"</a>" * 300
try:
    print("deep", len(list(etree.fromstring(deep).iter())))
except etree.XMLSyntaxError as e:
    print("deep", e.msg)

# ── the ElementTree API ─────────────────────────────────────────────────────────────────────────
el = etree.Element("doc", nsmap={"x": "urn:x"}, version="2")
child = etree.SubElement(el, "{urn:x}part", name="one")
child.text, child.tail = "text", "tail"
etree.SubElement(el, "part").append(etree.Comment(" note "))
el.insert(0, etree.ProcessingInstruction("pi", "data"))
el.set("{urn:x}flag", "yes")
print("built", etree.tostring(el))
print("find", [e.get("name") for e in el.findall("{urn:x}part")], el.findtext("{urn:x}part"), el.find("missing"))
print("elementpath", [e.tag for e in root.iterfind(".//{urn:a}item[@id='2']")], root.findall("{*}item")[0].get("id"),
      [e.get("id") for e in root.findall(".//{urn:a}item[@kind]")])
print("itertext", list(root.itertext()))
print("qname", etree.QName(child).localname, etree.QName(child).namespace, etree.QName("urn:q", "n").text)
copy = etree.fromstring(etree.tostring(root))
etree.strip_tags(copy, "{urn:b}item")
etree.cleanup_namespaces(copy)
print("strip", etree.tostring(copy)[:120])
etree.strip_elements(copy, "{urn:a}item", with_tail=False)
print("strip elements", etree.tostring(copy)[:120])
print("index", root.index(root[2]), len(root), root[1].getparent().tag, root[1].getnext().get("id"))

# ── XPath ───────────────────────────────────────────────────────────────────────────────────────
ns = {"a": "urn:a", "b": "urn:b"}
for expr in ["count(//a:item)", "//a:item/@id", "string(//a:item[2])", "sum(//@id)", "//b:item/text()",
             "name(/*)", "boolean(//a:empty)", "//a:item[last()]/@kind", "concat('x', 1 div 3)", "local-name(//*[@b:attr])",
             "translate('abc', 'abc', 'ABC')", "//a:item[contains(., 'welcome')]/@id", "1 div 0", "-1 div 0", "0 div 0",
             "number('x')", "round(2.5)", "floor(-1.5)", "string(0.1 + 0.2)", "string(1e21)"]:
    r = root.xpath(expr, namespaces=ns)
    print("xpath", expr, [str(x) for x in r] if isinstance(r, list) else r)
print("smart", root.xpath("//a:item/text()", namespaces=ns)[0].getparent().get("id"),
      root.xpath("//a:item/text()", namespaces=ns)[0].is_text)
ev = etree.XPathEvaluator(root, namespaces=ns)
print("evaluator", ev("count(//a:item)"), etree.XPath("$x * 2")(root, x=21))
print("exslt", root.xpath("re:test(string(//a:item[1]), '^hel+o')", namespaces={"re": "http://exslt.org/regular-expressions", **ns}),
      root.xpath("re:replace('a-b-c', '-', 'g', '+')", namespaces={"re": "http://exslt.org/regular-expressions"}),
      root.xpath("set:distinct(//a:item/@kind)", namespaces={"set": "http://exslt.org/sets", **ns}))
fns = etree.FunctionNamespace("urn:py")
fns["upper"] = lambda context, s: str(s).upper()
fns["sum"] = lambda context, nodes: sum(float(n) for n in nodes)
print("extension", root.xpath("p:upper(string(//a:item[1]))", namespaces={"p": "urn:py", **ns}),
      root.xpath("p:sum(//@id)", namespaces={"p": "urn:py"}))
for bad in ["//[", "unknown:f()", "count(", "$undefined"]:
    try:
        root.xpath(bad)
    except etree.XPathError as e:
        print("xpath error", bad, type(e).__name__, e)

# ── XSLT ────────────────────────────────────────────────────────────────────────────────────────
XSL = etree.XML(b"""<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:a="urn:a" xmlns:str="http://exslt.org/strings" xmlns:math="http://exslt.org/math"
    xmlns:exsl="http://exslt.org/common" xmlns:dyn="http://exslt.org/dynamic" extension-element-prefixes="exsl str math dyn">
  <xsl:param name="title" select="'none'"/>
  <xsl:output method="xml" indent="yes"/>
  <xsl:key name="by-kind" match="a:item" use="@kind"/>
  <xsl:template match="/">
    <xsl:message>transforming <xsl:value-of select="$title"/></xsl:message>
    <out title="{$title}" max="{math:max(//@id)}" n="{count(key('by-kind', 'plain'))}">
      <xsl:for-each select="//a:item"><xsl:sort select="@id" data-type="number" order="descending"/>
        <row pos="{position()}"><xsl:value-of select="translate(., 'abcdefghijklmnopqrstuvwxyz', 'ABCDEFGHIJKLMNOPQRSTUVWXYZ')"/></row>
      </xsl:for-each>
      <xsl:for-each select="str:tokenize('a,b,,c', ',')"><tok><xsl:value-of select="."/></tok></xsl:for-each>
      <xsl:variable name="frag"><x>1</x><x>2</x></xsl:variable>
      <total><xsl:value-of select="sum(exsl:node-set($frag)/x)"/></total>
      <dyn><xsl:value-of select="dyn:evaluate('1 + 2')"/></dyn>
      <num><xsl:value-of select="format-number(1234567.891, '#,##0.00')"/></num>
      <pad><xsl:value-of select="str:padding(3, '-')"/>|<xsl:value-of select="str:align('ab', '-----', 'right')"/></pad>
      <xsl:comment>generated</xsl:comment>
    </out>
  </xsl:template>
</xsl:stylesheet>""")
transform = etree.XSLT(XSL)
result = transform(tree, title=etree.XSLT.strparam("Report's"))
print("xslt", str(result))
print("xslt log", [(e.message, e.level_name) for e in transform.error_log])
text = etree.XSLT(etree.XML(b"""<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="text"/><xsl:template match="/">value: <xsl:value-of select="count(//*)"/></xsl:template></xsl:stylesheet>"""))
print("xslt text", str(text(tree)), bytes(text(tree)))
html_out = etree.XSLT(etree.XML(b"""<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
  <xsl:output method="html"/><xsl:template match="/"><html><body><br/><p>x &amp; y</p></body></html></xsl:template></xsl:stylesheet>"""))
print("xslt html", str(html_out(tree)))
for bad in [b"<xsl:stylesheet version='1.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'><xsl:template match='/'><xsl:bogus/></xsl:template></xsl:stylesheet>",
            b"<xsl:stylesheet version='1.0' xmlns:xsl='http://www.w3.org/1999/XSL/Transform'><xsl:template match='/'><xsl:message terminate='yes'>stop</xsl:message></xsl:template></xsl:stylesheet>"]:
    try:
        etree.XSLT(etree.XML(bad))(tree)
        print("xslt ran?")
    except (etree.XSLTParseError, etree.XSLTApplyError) as e:
        print("xslt error", type(e).__name__, e)
ac = etree.XSLTAccessControl(read_file=False, write_file=False, create_dir=False, read_network=False, write_network=False)
try:
    etree.XSLT(etree.XML(b"""<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform">
      <xsl:template match="/"><xsl:copy-of select="document('secret.xml')"/></xsl:template></xsl:stylesheet>"""), access_control=ac)(tree)
    print("access?")
except etree.XSLTApplyError as e:
    print("access", e)

# ── validation ──────────────────────────────────────────────────────────────────────────────────
dtd = etree.DTD(io.StringIO("<!ELEMENT list (entry+)><!ELEMENT entry (#PCDATA)><!ATTLIST entry n CDATA #REQUIRED>"))
for doc in [b"<list><entry n='1'>x</entry></list>", b"<list><entry>x</entry><other/></list>"]:
    ok = dtd.validate(etree.fromstring(doc))
    print("dtd", ok, errors(dtd.error_log))
XSD = b"""<xs:schema xmlns:xs="http://www.w3.org/2001/XMLSchema" targetNamespace="urn:s" xmlns="urn:s" elementFormDefault="qualified">
  <xs:element name="order"><xs:complexType><xs:sequence>
    <xs:element name="id" type="xs:positiveInteger"/>
    <xs:element name="code"><xs:simpleType><xs:restriction base="xs:string"><xs:pattern value="[A-Z]{2}-\\d{3}"/></xs:restriction></xs:simpleType></xs:element>
    <xs:element name="when" type="xs:date"/>
    <xs:element name="amount" type="xs:decimal" maxOccurs="3"/>
  </xs:sequence><xs:attribute name="state" use="required"><xs:simpleType><xs:restriction base="xs:token">
    <xs:enumeration value="open"/><xs:enumeration value="closed"/></xs:restriction></xs:simpleType></xs:attribute>
  </xs:complexType></xs:element></xs:schema>"""
schema = etree.XMLSchema(etree.XML(XSD))
for doc in [b"<order xmlns='urn:s' state='open'><id>4</id><code>AB-123</code><when>2026-10-15</when><amount>1.5</amount></order>",
            b"<order xmlns='urn:s' state='pending'><id>-4</id><code>ab-12</code><when>2026-13-40</when><amount>x</amount><extra/></order>"]:
    print("xsd", schema.validate(etree.fromstring(doc)), errors(schema.error_log))
try:
    schema.assertValid(etree.fromstring(b"<order xmlns='urn:s'/>"))
except etree.DocumentInvalid as e:
    print("assert", e)
try:
    etree.XMLSchema(etree.XML(b"<xs:schema xmlns:xs='http://www.w3.org/2001/XMLSchema'><xs:element name='a' type='nope'/></xs:schema>"))
except etree.XMLSchemaParseError as e:
    print("xsd parse", e)
rng = etree.RelaxNG(etree.XML(b"""<element name="book" xmlns="http://relaxng.org/ns/structure/1.0"
    datatypeLibrary="http://www.w3.org/2001/XMLSchema-datatypes">
  <attribute name="year"><data type="gYear"/></attribute>
  <oneOrMore><element name="chapter"><text/></element></oneOrMore></element>"""))
for doc in [b"<book year='2026'><chapter>one</chapter></book>", b"<book year='twenty'><preface/></book>"]:
    print("relaxng", rng.validate(etree.fromstring(doc)), errors(rng.error_log))
from lxml import isoschematron
sch = isoschematron.Schematron(etree.XML(b"""<schema xmlns="http://purl.oclc.org/dsdl/schematron">
  <pattern><rule context="item"><assert test="@price &gt; 0">price must be positive</assert>
  <report test="@price &gt; 100">expensive item</report></rule></pattern></schema>"""), store_report=True)
for doc in [b"<items><item price='5'/></items>", b"<items><item price='-1'/><item price='500'/></items>"]:
    ok = sch.validate(etree.fromstring(doc))
    print("schematron", ok, [e.message.split("\n")[0][:60] for e in sch.error_log][:3])
old_sch = etree.Schematron(etree.XML(b"""<schema xmlns="http://www.ascc.net/xml/schematron">
  <pattern name="p"><rule context="a"><assert test="b">a needs b</assert></rule></pattern></schema>"""))
print("schematron (libxml2)", old_sch.validate(etree.fromstring(b"<a><b/></a>")), old_sch.validate(etree.fromstring(b"<a/>")))

# ── objectify ───────────────────────────────────────────────────────────────────────────────────
o = objectify.fromstring(b"<data><count>3</count><ratio>0.25</ratio><flag>true</flag><name>x</name><none xsi:nil='true' "
                         b"xmlns:xsi='http://www.w3.org/2001/XMLSchema-instance'/><count>7</count></data>")
print("objectify", o.count + 1, o.ratio * 4, bool(o.flag), o.name + "y", o.none, [int(c) for c in o.count],
      type(o.count).__name__, type(o.ratio).__name__, type(o.flag).__name__)
o.added = 5
o.list = [1, "two", 3.0]
objectify.annotate(o)
print("annotated", etree.tostring(o)[:200])
objectify.deannotate(o, cleanup_namespaces=True)
print("dump", objectify.dump(o).splitlines()[:6])
path = objectify.ObjectPath("data.count[1]")
print("objectpath", path(o), path.hasattr(o))
maker = objectify.ElementMaker(annotate=False)
print("maker", etree.tostring(maker.root(maker.a(1), maker.b("x"))))

# ── the E-factory, SAX ──────────────────────────────────────────────────────────────────────────
page = E.html(E.head(E.title("T")), E.body(E.h1("Head", {"class": "big"}), E.p("text ", E.b("bold"), " more")))
print("builder", etree.tostring(page))
NSM = ElementMaker(namespace="urn:m", nsmap={"m": "urn:m"})
print("builder ns", etree.tostring(NSM.doc(NSM.part("x", id="1"))))
print("html builder", lxml.html.tostring(H.DIV(H.CLASS("c"), H.P("para"), H.A("link", href="/x"))))
handler = sax.ElementTreeContentHandler()
sax.saxify(root, handler)
print("sax", etree.tostring(handler.etree.getroot())[:80])

events = []
from xml.sax.handler import ContentHandler
class Record(ContentHandler):
    def startElementNS(self, name, qname, attrs):
        events.append(("start", name, sorted(attrs.getNames())))
    def characters(self, data):
        if data.strip():
            events.append(("chars", data))
sax.saxify(etree.fromstring(b"<a xmlns='urn:z' k='v'><b>t</b></a>"), Record())
print("sax events", events)

# ── iterparse, feed and pull parsers, targets ───────────────────────────────────────────────────
with open("doc.xml", "wb") as f:
    f.write(DOC)
def node(el):
    if not isinstance(el, etree._Element):
        return el
    return el.tag if isinstance(el.tag, str) else (type(el).__name__, el.text)
print("iterparse", [(ev, node(el)) for ev, el in etree.iterparse("doc.xml", events=("start", "end", "start-ns", "comment", "pi"))][:12])
print("iterparse tag", [el.get("id") for _, el in etree.iterparse("doc.xml", tag="{urn:a}item")])
parser = etree.XMLParser()
for chunk in [b"<r><a>1</a>", b"<a>2", b"</a></r>"]:
    parser.feed(chunk)
print("feed", etree.tostring(parser.close()))
pull = etree.XMLPullParser(events=("start", "end"))
pull.feed(b"<r><x/>")
first = [(e, el.tag) for e, el in pull.read_events()]
pull.feed(b"<y/></r>")
print("pull", first, [(e, el.tag) for e, el in pull.read_events()])
class Target:
    def __init__(self):
        self.log = []
    def start(self, tag, attrib):
        self.log.append(("start", tag, dict(attrib)))
    def end(self, tag):
        self.log.append(("end", tag))
    def data(self, data):
        self.log.append(("data", data))
    def close(self):
        return self.log
print("target", etree.fromstring(b"<a x='1'>t<b/></a>", etree.XMLParser(target=Target())))
print("html target", etree.fromstring(b"<p>unclosed<br>x", etree.HTMLParser(target=Target())))

# ── xmlfile, gzip, XInclude, resolvers, C14N ────────────────────────────────────────────────────
out = io.BytesIO()
with etree.xmlfile(out, encoding="utf-8") as xf:
    xf.write_declaration(standalone=True)
    with xf.element("log", nsmap={None: "urn:log"}):
        for i in range(3):
            xf.write(etree.Element("entry", n=str(i)), pretty_print=True)
            xf.flush()
        xf.write("text & more")
print("xmlfile", out.getvalue())
tree.write("doc.xml.gz", compression=9)
with gzip.open("doc.xml.gz") as f:
    print("gzip written", md5(f.read()))
print("gzip parsed", etree.parse("doc.xml.gz").getroot().tag, len(etree.parse("doc.xml.gz").getroot()))
with open("part.xml", "wb") as f:
    f.write(b"<part>included</part>")
with open("main.xml", "wb") as f:
    f.write(b"<main xmlns:xi='http://www.w3.org/2001/XInclude'><xi:include href='part.xml'/>"
            b"<xi:include href='part.xml' parse='text'/><xi:include href='missing.xml'><xi:fallback>none</xi:fallback></xi:include></main>")
inc = etree.parse("main.xml")
inc.xinclude()
print("xinclude", etree.tostring(inc))
class Resolver(etree.Resolver):
    def resolve(self, url, pubid, context):
        if url.startswith("mem:"):
            return self.resolve_string(f"<from url='{url}'/>", context)
r = etree.XMLParser(load_dtd=True, no_network=True, resolve_entities=True)   # (lxml 5 on: internal ones only by default)
r.resolvers.add(Resolver())
print("resolver", etree.tostring(etree.fromstring(b"<!DOCTYPE d [<!ENTITY e SYSTEM 'mem:thing'>]><d>&e;</d>", r)))
C14N_DOC = b"<n:a xmlns:n='urn:n' xmlns:unused='urn:u' z='1' a='2'><!-- c --><b   x='y'  />text</n:a>"
print("c14n", etree.tostring(etree.fromstring(C14N_DOC), method="c14n"),
      etree.tostring(etree.fromstring(C14N_DOC), method="c14n", exclusive=True, with_comments=False))
print("c14n2", etree.canonicalize(C14N_DOC.decode(), with_comments=True, strip_text=True))

# ── HTML ────────────────────────────────────────────────────────────────────────────────────────
HTML = b"""<html><head><title>Test &amp; page</title><meta charset="utf-8"></head>
<body><div id=main class="a b"><p>One<p>Two <b>bold<i>both</b> italic</i><br>
<a href="/rel">link</a><img src=x.png alt=pic><table><tr><td>cell</table>
<script>if (a < b) {}</script><!-- comment --></div></body></html>"""
doc = lxml.html.fromstring(HTML)
print("html", doc.tag, doc.find(".//title").text, [p.text_content() for p in doc.iter("p")])
print("html out", lxml.html.tostring(doc))
print("html links", [(el.tag, attr, link) for el, attr, link, pos in doc.iterlinks()])
doc.make_links_absolute("https://example.org/base/")
print("absolute", [link for _, _, link, _ in doc.iterlinks()])
print("classes", sorted(doc.get_element_by_id("main").classes), doc.find_class("b")[0].get("id"))
print("fragment", [lxml.html.tostring(f) for f in lxml.html.fragments_fromstring("<p>a</p>tail<p>b</p>")[1:]])
print("html parse", etree.tostring(etree.fromstring(b"<p>x<div>y</p>", etree.HTMLParser())))
print("html encodings", lxml.html.fromstring("<p>Grüße</p>".encode("latin-1"), parser=lxml.html.HTMLParser(encoding="iso-8859-1")).text_content(),
      etree.fromstring(b'<html><head><meta charset="shift_jis"></head><body>' + "日本".encode("shift_jis") + b"</body></html>",
                       etree.HTMLParser()).findtext(".//body"))
form = lxml.html.fromstring(b"<form action='/s'><input name=q value=x><select name=s><option value=1 selected>a<option value=2>b</select>"
                            b"<input type=checkbox name=c value=on checked><textarea name=t>txt</textarea></form>")
print("form", sorted(form.form_values()), form.inputs["s"].value, sorted(form.fields.keys()))
print("htmldiff", lxml.html.diff.htmldiff("<p>The quick brown fox</p><p>jumps</p>", "<p>The slow brown fox</p><p>jumps high</p>"))
print("html_annotate", lxml.html.diff.html_annotate([("<p>one two</p>", "v1"), ("<p>one three</p>", "v2")]))
try:
    import lxml.html.clean
    print("clean", "importable")
except ImportError as e:
    print("clean", type(e).__name__, str(e)[:60])

# ── odds and ends ───────────────────────────────────────────────────────────────────────────────
print("version strings", etree.__version__, lxml.__version__ if hasattr(lxml, "__version__") else None)
try:
    etree.Element("bad name")
except ValueError as e:
    print("bad name", e)
try:
    etree.Element("a").set("x", "\x01")
except ValueError as e:
    print("bad value", e)
print("entities", etree.tostring(etree.Entity("amp")), etree.tostring(etree.fromstring("<a>é&#x1F600;</a>"), encoding="ascii"))
print("memory", sum(len(etree.tostring(etree.fromstring(b"<a>" + b"<b/>" * 1000 + b"</a>"))) for _ in range(200)))
print("done")

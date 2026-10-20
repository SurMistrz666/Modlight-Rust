#!/usr/bin/env python3
"""Builds the .docx parity corpus for docx_parity.py.

    python3 server/tests/parity_media/docx_corpus.py <out dir>

Families (file name prefixes):
  f-*   hand-built OOXML exercising one mammoth feature each
  i-*   embedded pictures (PNG, JPEG with EXIF orientation, 24 Mpx edge, EMF,
        enough to exhaust the 3 MB budget)
  h-*   hostile archives (bombs, ZIP64 markers, data descriptors, duplicate
        names, prepended data, encrypted flags, ...)
  x-*   XML parser edge cases (xmldom's leniencies and its errors)
  t-*   written by a real producer: macOS textutil from HTML and RTF
  iso-* run in a reference process of their own (they pollute
        Object.prototype or never finish)

Stdlib only, plus `textutil` (macOS) and the `vips` CLI for JPEG/GIF.
"""
import os
import random
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib

W_NS = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R_NS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
REL_NS = "http://schemas.openxmlformats.org/package/2006/relationships"
CT_NS = "http://schemas.openxmlformats.org/package/2006/content-types"
WP_NS = "http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"
A_NS = "http://schemas.openxmlformats.org/drawingml/2006/main"
PIC_NS = "http://schemas.openxmlformats.org/drawingml/2006/picture"
MC_NS = "http://schemas.openxmlformats.org/markup-compatibility/2006"
V_NS = "urn:schemas-microsoft-com:vml"
O_NS = "urn:schemas-microsoft-com:office:office"
W14_NS = "http://schemas.microsoft.com/office/word/2010/wordml"
WPS_NS = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape"
RT = "http://schemas.openxmlformats.org/officeDocument/2006/relationships/"

NS_DECL = (f'xmlns:w="{W_NS}" xmlns:r="{R_NS}" xmlns:wp="{WP_NS}" xmlns:a="{A_NS}" '
           f'xmlns:pic="{PIC_NS}" xmlns:mc="{MC_NS}" xmlns:v="{V_NS}" xmlns:o="{O_NS}" '
           f'xmlns:w14="{W14_NS}" xmlns:wps="{WPS_NS}"')


# ---------------------------------------------------------------------------
# A ZIP writer that can lie
# ---------------------------------------------------------------------------

def deflate_raw(data, level=6):
    c = zlib.compressobj(level, zlib.DEFLATED, -15)
    return c.compress(data) + c.flush()


class Member:
    def __init__(self, name, body, method=8, **kw):
        self.name = name if isinstance(name, bytes) else name.encode("utf-8")
        self.body = body if isinstance(body, bytes) else body.encode("utf-8")
        self.method = method
        self.data = kw.get("data")              # raw stored bytes (overrides compression)
        self.declared = kw.get("declared")      # uncompressed size written (both headers)
        self.compressed = kw.get("compressed")  # compressed size written (both headers)
        self.flags = kw.get("flags", 0)
        self.local_name = kw.get("local_name")  # a different name in the local header
        self.central_extra = kw.get("central_extra", b"")
        self.local_extra = kw.get("local_extra", b"")
        self.external = kw.get("external", 0)
        self.made_by = kw.get("made_by", 20)
        self.crc = kw.get("crc")
        self.descriptor = kw.get("descriptor", False)  # write a data descriptor, zero local sizes


def build_zip(members, prepend=b"", count=None, comment=b"", comment_len=None, extra_central=(),
              eocd_fields=None, between=b""):
    """members -> bytes. `extra_central`: members written as central records
    AFTER the declared ones (count stays len(members)). `between`: bytes
    between the central directory and the trailer."""
    out = bytearray(prepend)
    centrals = []
    for m in list(members) + list(extra_central):
        data = m.data if m.data is not None else (deflate_raw(m.body) if m.method == 8 else m.body)
        crc = m.crc if m.crc is not None else zlib.crc32(m.body)
        declared = m.declared if m.declared is not None else len(m.body)
        csize = m.compressed if m.compressed is not None else len(data)
        lname = m.local_name if m.local_name is not None else m.name
        flags = m.flags | (0x08 if m.descriptor else 0)
        offset = len(out) - len(prepend)
        local = struct.pack("<IHHHHHIIIHH", 0x04034B50, 20, flags, m.method, 0, 0x21,
                            0 if m.descriptor else crc, 0 if m.descriptor else csize,
                            0 if m.descriptor else declared, len(lname), len(m.local_extra))
        out += local + lname + m.local_extra + data
        if m.descriptor:
            out += struct.pack("<IIII", 0x08074B50, crc, csize, declared)
        central = struct.pack("<IHHHHHHIIIHHHHHII", 0x02014B50, m.made_by, 20, flags, m.method, 0, 0x21,
                              crc, csize, declared, len(m.name), len(m.central_extra), 0, 0, 0,
                              m.external, offset)
        centrals.append(central + m.name + m.central_extra)
    cd_offset = len(out) - len(prepend)
    cd = b"".join(centrals)
    out += cd + between
    n = len(members) if count is None else count
    f = dict(disk=0, disk_cd=0, on_disk=n, total=n, size=len(cd), offset=cd_offset)
    if eocd_fields:
        f.update(eocd_fields)
    out += struct.pack("<IHHHHIIH", 0x06054B50, f["disk"], f["disk_cd"], f["on_disk"], f["total"],
                       f["size"], f["offset"], len(comment) if comment_len is None else comment_len)
    out += comment
    return bytes(out)


# ---------------------------------------------------------------------------
# Documents
# ---------------------------------------------------------------------------

def content_types(extra_defaults=(), overrides=()):
    d = "".join(f'<Default Extension="{e}" ContentType="{t}"/>' for e, t in
                [("xml", "application/xml"), ("rels", "application/vnd.openxmlformats-package.relationships+xml"),
                 ("png", "image/png"), ("jpeg", "image/jpeg"), ("jpg", "image/jpeg"), ("gif", "image/gif"),
                 ("emf", "image/x-emf")] + list(extra_defaults))
    o = "".join(f'<Override PartName="{p}" ContentType="{t}"/>' for p, t in
                [("/word/document.xml",
                  "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml")]
                + list(overrides))
    return f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Types xmlns="{CT_NS}">{d}{o}</Types>'


def rels(items):
    body = "".join(f'<Relationship Id="{i}" Type="{t if t.startswith("http") else RT + t}" Target="{tg}"'
                   + (' TargetMode="External"' if ext else "") + "/>"
                   for i, t, tg, ext in items)
    return f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Relationships xmlns="{REL_NS}">{body}</Relationships>'


def document(body, ns=NS_DECL):
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n'
            f'<w:document {ns}><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/></w:sectPr>'
            f'</w:body></w:document>')


def part_xml(root, inner, ns=NS_DECL):
    return f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n<{root} {ns}>{inner}</{root}>'


STYLES = part_xml("w:styles", "".join([
    '<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>',
    *[f'<w:style w:type="paragraph" w:styleId="Heading{i}"><w:name w:val="heading {i}"/></w:style>'
      for i in range(1, 7)],
    '<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/></w:style>',
    '<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/></w:style>',
    '<w:style w:type="paragraph" w:styleId="MyHead"><w:name w:val="Heading 2"/></w:style>',
    '<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/></w:style>',
    '<w:style w:type="paragraph" w:styleId="FootnoteText"><w:name w:val="footnote text"/></w:style>',
    '<w:style w:type="paragraph" w:styleId="Body"><w:name w:val="Body"/></w:style>',
    '<w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/></w:style>',
    '<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/></w:style>',
    '<w:style w:type="character" w:styleId="FootnoteReference"><w:name w:val="footnote reference"/></w:style>',
    '<w:style w:type="character" w:styleId="Emphasis"><w:name w:val="Emphasis"/></w:style>',
    '<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/></w:style>',
    '<w:style w:type="numbering" w:styleId="BulletStyle"><w:name w:val="Bullet Style"/>'
    '<w:pPr><w:numPr><w:numId w:val="3"/></w:numPr></w:pPr></w:style>',
    '<w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Duplicate Normal"/></w:style>',
]))


def numbering_xml(extra=""):
    def lvl(i, fmt, pstyle=None):
        ps = f'<w:pStyle w:val="{pstyle}"/>' if pstyle else ""
        return f'<w:lvl w:ilvl="{i}"><w:start w:val="1"/><w:numFmt w:val="{fmt}"/>{ps}</w:lvl>'
    bullets = "".join(lvl(i, "bullet") for i in range(9))
    decimals = "".join(lvl(i, "decimal" if i % 2 == 0 else "lowerLetter") for i in range(9))
    return part_xml("w:numbering", "".join([
        f'<w:abstractNum w:abstractNumId="0">{bullets}</w:abstractNum>',
        f'<w:abstractNum w:abstractNumId="1">{decimals}</w:abstractNum>',
        '<w:abstractNum w:abstractNumId="2"><w:numStyleLink w:val="BulletStyle"/></w:abstractNum>',
        f'<w:abstractNum w:abstractNumId="3">{lvl(0, "decimal", "ListNumberStyle")}{lvl(1, "bullet")}</w:abstractNum>',
        '<w:abstractNum w:abstractNumId="4"><w:lvl><w:numFmt w:val="decimal"/></w:lvl></w:abstractNum>',
        '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>',
        '<w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>',
        '<w:num w:numId="3"><w:abstractNumId w:val="0"/></w:num>',
        '<w:num w:numId="4"><w:abstractNumId w:val="2"/></w:num>',
        '<w:num w:numId="5"><w:abstractNumId w:val="3"/></w:num>',
        '<w:num w:numId="6"><w:abstractNumId w:val="4"/></w:num>',
        '<w:num w:numId="7"><w:abstractNumId w:val="99"/></w:num>',
        extra,
    ]))


def docx(body, doc_rels=(), parts=None, styles=True, numbering=False, extra_ct=(), overrides=(),
         package_rels=None, document_xml=None, members_kw=None, method=8):
    """A complete .docx. `parts`: {name: bytes|str} added as they are."""
    files = {}
    files["[Content_Types].xml"] = content_types(extra_ct, overrides)
    files["_rels/.rels"] = rels(package_rels or [("rId1", "officeDocument", "word/document.xml", False)])
    files["word/document.xml"] = document_xml if document_xml is not None else document(body)
    r = list(doc_rels)
    if styles:
        files["word/styles.xml"] = STYLES if styles is True else styles
        r.append(("rIdStyles", "styles", "styles.xml", False))
    if numbering:
        files["word/numbering.xml"] = numbering_xml() if numbering is True else numbering
        r.append(("rIdNumbering", "numbering", "numbering.xml", False))
    if r:
        files["word/_rels/document.xml.rels"] = rels(r)
    for k, v in (parts or {}).items():
        if v is None:
            files.pop(k, None)
        else:
            files[k] = v
    kw = members_kw or {}
    return build_zip([Member(n, b, method=method, **kw.get(n, {})) for n, b in files.items()])


def p(text="", style=None, runs=None, ppr=""):
    props = (f'<w:pStyle w:val="{style}"/>' if style else "") + ppr
    pp = f"<w:pPr>{props}</w:pPr>" if props else ""
    return f"<w:p>{pp}{runs if runs is not None else r(text)}</w:p>"


def r(text, rpr=""):
    rp = f"<w:rPr>{rpr}</w:rPr>" if rpr else ""
    return f'<w:r>{rp}<w:t xml:space="preserve">{text}</w:t></w:r>'


def li(text, num_id, ilvl=None):
    lv = f'<w:ilvl w:val="{ilvl}"/>' if ilvl is not None else ""
    return p(runs=r(text), ppr=f'<w:numPr>{lv}<w:numId w:val="{num_id}"/></w:numPr>')


# ---------------------------------------------------------------------------
# f-*: one mammoth feature each
# ---------------------------------------------------------------------------

def features():
    out = {}
    out["f-basic.docx"] = docx(
        p("Plain paragraph.") + p("Title text", "Title") + p("Heading by id", "Heading1")
        + p("Heading two by id", "Heading2") + p("Heading 3", "Heading3") + p("Deep", "Heading6")
        + p("Heading by style name", "MyHead") + p("Normal style", "Normal") + p("Quote", "Quote")
        + p("Undefined style", "NoSuchStyle") + p("") + p(runs="") + p("Body", "Body")
        + "<w:p/>" + p("After empty"))
    out["f-runs.docx"] = docx(p(runs="".join([
        r("bold ", "<w:b/>"), r("not bold ", '<w:b w:val="0"/>'), r("false bold ", '<w:b w:val="false"/>'),
        r("italic ", "<w:i/>"), r("both ", "<w:b/><w:i/>"), r("under ", '<w:u w:val="single"/>'),
        r("under none ", '<w:u w:val="none"/>'), r("strike ", "<w:strike/>"), r("caps ", "<w:caps/>"),
        r("small ", "<w:smallCaps/>"), r("sup", '<w:vertAlign w:val="superscript"/>'),
        r("sub", '<w:vertAlign w:val="subscript"/>'), r(" base ", '<w:vertAlign w:val="baseline"/>'),
        r("hi ", '<w:highlight w:val="yellow"/>'), r("strong style ", '<w:rStyle w:val="Strong"/>'),
        r("emph style ", '<w:rStyle w:val="Emphasis"/>'), r("unknown style ", '<w:rStyle w:val="Nope"/>'),
        r("all ", '<w:b/><w:i/><w:strike/><w:vertAlign w:val="superscript"/><w:rStyle w:val="Strong"/>'),
        r("bold again", "<w:b/>"), r(" bold more", "<w:b/>"),
    ])))
    out["f-text.docx"] = docx(
        p(runs='<w:r><w:t>a</w:t><w:tab/><w:t>b</w:t><w:br/><w:t>c</w:t><w:br w:type="page"/>'
          '<w:t>d</w:t><w:br w:type="column"/><w:t>e</w:t><w:br w:type="textWrapping"/>'
          '<w:br w:type="unknown"/><w:noBreakHyphen/><w:softHyphen/><w:t>f</w:t></w:r>')
        + p("Escapes: &amp; &lt; &gt; &quot; &apos; \" ' > ")
        + p(runs='<w:r><w:t xml:space="preserve">  spaces  kept  </w:t></w:r>')
        + p(runs='<w:r><w:t>refs &#65;&#x42;&#x1F600;&#169;&#xD83D;&#xDE00;</w:t></w:r>')
        + p(runs='<w:r><w:t><![CDATA[cdata <b>]]></w:t></w:r>')
        + p(runs='<w:r><w:t>one</w:t><w:t>two</w:t></w:r><w:r><w:t></w:t></w:r><w:r><w:t/></w:r>')
        + p(runs='<w:r><w:sym w:font="Symbol" w:char="F0A7"/><w:sym w:font="Wingdings" w:char="F0FC"/>'
                 '<w:sym w:font="Symbol" w:char="61"/><w:sym w:font="Arial" w:char="F041"/>'
                 '<w:sym w:font="wingdings 3" w:char="E9"/><w:sym w:font="Symbol"/></w:r>')
        + p(runs='<w:r><w:t>emoji 😀 é 中文 עברית</w:t></w:r>')
        + p(runs='<w:r><w:lastRenderedPageBreak/><w:t>after ignorables</w:t></w:r><w:proofErr/>'))
    out["f-sym-nofont.docx"] = docx(p(runs='<w:r><w:sym w:char="F041"/></w:r>'))
    out["f-links.docx"] = docx(
        p(runs='<w:hyperlink r:id="rIdL1"><w:r><w:t>external</w:t></w:r></w:hyperlink>')
        + p(runs='<w:hyperlink w:anchor="bm1"><w:r><w:t>anchor</w:t></w:r></w:hyperlink>')
        + p(runs='<w:hyperlink r:id="rIdL2" w:anchor="frag"><w:r><w:t>both</w:t></w:r></w:hyperlink>')
        + p(runs='<w:hyperlink r:id="rIdL1" w:tgtFrame="_blank"><w:r><w:t>blank</w:t></w:r>'
                 '<w:r><w:rPr><w:b/></w:rPr><w:t> bold part</w:t></w:r></w:hyperlink>')
        + p(runs='<w:hyperlink><w:r><w:t>no target</w:t></w:r></w:hyperlink>')
        + p(runs='<w:hyperlink r:id="rIdL3"><w:r><w:rPr><w:rStyle w:val="Hyperlink"/></w:rPr>'
                 '<w:t>javascript</w:t></w:r></w:hyperlink>')
        + p(runs='<w:hyperlink r:id="rIdL1"><w:r><w:t>a</w:t></w:r></w:hyperlink>'
                 '<w:hyperlink r:id="rIdL1"><w:r><w:t>b</w:t></w:r></w:hyperlink>')
        + p(runs='<w:hyperlink r:id="rIdEmptyTarget"><w:r><w:t>no Target attribute</w:t></w:r></w:hyperlink>')
        + p(runs='<w:hyperlink r:id="rIdMissing"></w:hyperlink><w:r><w:t>empty link to missing rel</w:t></w:r>')
        + '<w:p><w:bookmarkStart w:id="0" w:name="bm1"/><w:r><w:t>target</w:t></w:r>'
          '<w:bookmarkEnd w:id="0"/><w:bookmarkStart w:id="1" w:name="_GoBack"/><w:bookmarkStart w:id="2"/></w:p>',
        doc_rels=[("rIdL1", "hyperlink", "http://example.com/a?b=1&amp;c=2", True),
                  ("rIdL2", "hyperlink", "http://example.com/page#old", True),
                  ("rIdL3", "hyperlink", "javascript:alert(&quot;x&quot;)", True),
                  ("rIdEmptyTarget", "hyperlink", "", True)])
    out["f-link-missing-rel.docx"] = docx(
        p(runs='<w:hyperlink r:id="rIdNone"><w:r><w:t>missing</w:t></w:r></w:hyperlink>'))
    out["f-link-missing-rel-anchor.docx"] = docx(
        p(runs='<w:hyperlink r:id="rIdNone" w:anchor="x"><w:r><w:t>missing</w:t></w:r></w:hyperlink>'))
    field = lambda instr, text, extra="": (
        f'<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve">{instr}</w:instrText></w:r>'
        f'<w:r><w:fldChar w:fldCharType="separate"/></w:r>{extra}<w:r><w:t>{text}</w:t></w:r>'
        f'<w:r><w:fldChar w:fldCharType="end"/></w:r>')
    out["f-fields.docx"] = docx(
        p(runs=field(' HYPERLINK "http://example.com/x" ', "quoted"))
        + p(runs=field('HYPERLINK http://example.com/y', "unquoted"))
        + p(runs=field(' HYPERLINK \\l "anchor1" ', "local"))
        + p(runs=field(' HYPERLINK \\l anchor2 ', "local unquoted"))
        + p(runs=field(' HYPERLINK "http://example.com/z" \\o "tip" ', "with tooltip"))
        + p(runs=field(' HYPERLINK "" ', "empty href"))
        + p(runs=field(' PAGE ', "3"))
        + p(runs=field(' HYPERLINK "http://outer" ', "outer", field(' HYPERLINK "http://inner" ', "inner")))
        + p(runs='<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>HYPER</w:instrText></w:r>'
                 '<w:r><w:instrText>LINK "http://split"</w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>'
                 '<w:r><w:t>split</w:t></w:r><w:r><w:t> two runs</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>')
        + p(runs='<w:fldSimple w:instr="PAGE"><w:r><w:t>simple</w:t></w:r></w:fldSimple>')
        + p(runs='<w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:checkBox><w:default w:val="1"/></w:checkBox>'
                 '</w:ffData></w:fldChar></w:r><w:r><w:instrText> FORMCHECKBOX </w:instrText></w:r>'
                 '<w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t> default on</w:t></w:r>')
        + p(runs='<w:r><w:fldChar w:fldCharType="begin"><w:ffData><w:checkBox><w:default w:val="1"/>'
                 '<w:checked w:val="0"/></w:checkBox></w:ffData></w:fldChar></w:r>'
                 '<w:r><w:instrText> FORMCHECKBOX </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r>'
                 '<w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t> checked off</w:t></w:r>')
        + p(runs=field(' HYPERLINK \t', "tab only")))
    out["f-field-end-first.docx"] = docx(p(runs='<w:r><w:fldChar w:fldCharType="end"/></w:r>'))
    out["f-field-separate-first.docx"] = docx(p(runs='<w:r><w:fldChar w:fldCharType="separate"/></w:r>'))
    out["f-field-unterminated.docx"] = docx(
        p(runs='<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText>HYPERLINK "http://open"</w:instrText></w:r>'
               '<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>never ends</w:t></w:r>')
        + p("next paragraph still linked"))

    lists = (li("bullet 1", 1, 0) + li("bullet 1.1", 1, 1) + li("bullet 1.1.1", 1, 2) + li("bullet 1.2", 1, 1)
             + li("bullet 2", 1, 0) + p("interrupting paragraph") + li("number 1", 2, 0) + li("letter a", 2, 1)
             + li("number 1.a.i", 2, 2) + li("level 3", 2, 3) + li("level 4", 2, 4) + li("level 5", 2, 5)
             + li("level 6 (beyond the map)", 2, 6) + li("number 2", 2, 0) + li("via style link", 4, 0)
             + li("no ilvl", 2) + li("no ilvl no index level", 6) + li("missing abstract", 7, 0)
             + li("missing num", 99, 0) + p("list by style", "ListNumberStyle") + li("mixed: bullet under number", 5, 1)
             + p(runs=r("ilvl with spaces"), ppr='<w:numPr><w:ilvl w:val=" 1 "/><w:numId w:val="2"/></w:numPr>')
             + p("styled list item", "ListParagraph", ppr='<w:numPr><w:ilvl w:val="0"/><w:numId w:val="1"/></w:numPr>'))
    out["f-lists.docx"] = docx(lists, numbering=True)
    out["f-lists-nonumbering.docx"] = docx(li("item without numbering part", 1, 0))
    out["f-num-without-abstract.docx"] = docx(
        p("x"), numbering=part_xml("w:numbering", '<w:num w:numId="1"/>'))
    out["f-numstylelink-missing.docx"] = docx(
        li("x", 1, 0), numbering=part_xml("w:numbering",
                                          '<w:abstractNum w:abstractNumId="0"><w:numStyleLink w:val="Nope"/></w:abstractNum>'
                                          '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>'))
    out["f-numstylelink-nostyles.docx"] = docx(
        li("x", 1, 0), styles=False, numbering=part_xml(
            "w:numbering", '<w:abstractNum w:abstractNumId="0"><w:numStyleLink w:val="Nope"/></w:abstractNum>'
                           '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>'))
    out["f-numstylelink-cycle.docx"] = docx(
        li("cycle", 1, 0), styles=part_xml("w:styles",
                                           '<w:style w:type="numbering" w:styleId="S"><w:pPr><w:numPr><w:numId w:val="1"/>'
                                           '</w:numPr></w:pPr></w:style>'),
        numbering=part_xml("w:numbering",
                           '<w:abstractNum w:abstractNumId="0"><w:numStyleLink w:val="S"/></w:abstractNum>'
                           '<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>'))

    cell = lambda text, tcpr="": f'<w:tc>{"<w:tcPr>" + tcpr + "</w:tcPr>" if tcpr else ""}{p(text)}</w:tc>'
    row = lambda cells, trpr="": f'<w:tr>{"<w:trPr>" + trpr + "</w:trPr>" if trpr else ""}{cells}</w:tr>'
    out["f-tables.docx"] = docx(
        '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/></w:tblPr><w:tblGrid><w:gridCol w:w="100"/></w:tblGrid>'
        + row(cell("H1") + cell("H2"), "<w:tblHeader/>") + row(cell("a") + cell("b")) + row(cell("c") + cell("d"))
        + "</w:tbl>"
        + p("merges")
        + "<w:tbl>" + row(cell("span 2", '<w:gridSpan w:val="2"/>') + cell("x"))
        + row(cell("v start", '<w:vMerge w:val="restart"/>') + cell("y") + cell("z"))
        + row(cell("", "<w:vMerge/>") + cell("y2") + cell("z2"))
        + row(cell("", '<w:vMerge w:val="continue"/>') + cell("y3") + cell("z3", '<w:vMerge w:val="restart"/>'))
        + row(cell("deleted row"), "<w:del/>") + row(cell("q") + cell("r") + cell("s", "<w:vMerge/>"))
        + "</w:tbl>"
        + "<w:tbl>" + row(cell("only header"), "<w:tblHeader/>") + "</w:tbl>"
        + "<w:tbl>" + row(cell("body") ) + row(cell("header after body"), "<w:tblHeader/>") + "</w:tbl>"
        + "<w:tbl>" + row("<w:tc>" + p("outer") + "<w:tbl>" + row(cell("inner")) + "</w:tbl></w:tc>") + "</w:tbl>"
        + "<w:tbl></w:tbl>" + "<w:tbl>" + row("") + "</w:tbl>"
        + "<w:tbl>" + row(cell("multi") + "<w:tc>" + p("p1") + p("p2") + "</w:tc>") + "</w:tbl>"
        + "<w:tbl>" + p("paragraph in table") + row(cell("after") + cell("m", "<w:vMerge/>")) + "</w:tbl>"
        + "<w:tbl>" + row(p("paragraph in row") + cell("cell")) + "</w:tbl>"
        + "<w:tbl>" + row(cell("gs0", '<w:gridSpan w:val="0"/>') + cell("gsneg", '<w:gridSpan w:val="-1"/>')
                          + cell("gsabc", '<w:gridSpan w:val="abc"/>') + cell("gs25", '<w:gridSpan w:val="2.5"/>')
                          + cell("gsbig", '<w:gridSpan w:val="123456789012345678901234"/>')
                          + cell("gsinf", '<w:gridSpan w:val="' + "9" * 400 + '"/>'))
        + row(cell("m0", "<w:vMerge/>") + cell("m1", "<w:vMerge/>") + cell("m2", "<w:vMerge/>")) + "</w:tbl>"
        + "<w:tc>" + p("stray cell") + "</w:tc>" + "<w:tr>" + cell("stray row") + "</w:tr>")
    out["f-table-style-nostyles.docx"] = docx(
        '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/></w:tblPr><w:tr>' + cell("x") + "</w:tr></w:tbl>",
        styles=False)
    out["f-table-style-empty-nostyles.docx"] = docx(
        '<w:tbl><w:tblPr><w:tblStyle w:val=""/></w:tblPr><w:tr>' + cell("x") + "</w:tr></w:tbl>", styles=False)

    notes_body = (p(runs=r("Text with footnote") + '<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr>'
                         '<w:footnoteReference w:id="1"/></w:r>' + r(" and endnote")
                    + '<w:r><w:endnoteReference w:id="1"/></w:r>' + r(" and again")
                    + '<w:r><w:footnoteReference w:id="2"/></w:r><w:r><w:footnoteReference w:id="1"/></w:r>'))
    footnotes = part_xml("w:footnotes", "".join([
        '<w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>',
        '<w:footnote w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>',
        '<w:footnote w:id="1"><w:p><w:pPr><w:pStyle w:val="FootnoteText"/></w:pPr><w:r><w:footnoteRef/></w:r>'
        '<w:r><w:t xml:space="preserve"> First note.</w:t></w:r></w:p></w:footnote>',
        '<w:footnote w:id="2"><w:p><w:r><w:t>Second note with </w:t></w:r><w:hyperlink r:id="rIdF1">'
        '<w:r><w:t>a link</w:t></w:r></w:hyperlink><w:r><w:footnoteReference w:id="1"/></w:r></w:p>'
        '<w:p><w:r><w:t>second paragraph</w:t></w:r></w:p></w:footnote>',
        '<w:footnote w:id="3"><w:p><w:r><w:t>Unreferenced.</w:t></w:r></w:p></w:footnote>',
    ]))
    endnotes = part_xml("w:endnotes", '<w:endnote w:id="1"><w:p><w:r><w:t>End note.</w:t></w:r></w:p></w:endnote>')
    note_parts = {"word/footnotes.xml": footnotes, "word/endnotes.xml": endnotes,
                  "word/_rels/footnotes.xml.rels": rels([("rIdF1", "hyperlink", "http://example.com/fn", True)])}
    out["f-notes.docx"] = docx(notes_body, parts=note_parts,
                               doc_rels=[("rIdFn", "footnotes", "footnotes.xml", False),
                                         ("rIdEn", "endnotes", "endnotes.xml", False)])
    out["f-note-missing.docx"] = docx(p(runs=r("x") + '<w:r><w:footnoteReference w:id="7"/></w:r>'))
    out["f-notes-fallback-path.docx"] = docx(notes_body, parts=note_parts)

    comments = part_xml("w:comments", "".join([
        '<w:comment w:id="0" w:author="Ann Author" w:initials="AA"><w:p><w:r><w:t>First comment.</w:t></w:r></w:p></w:comment>',
        '<w:comment w:id="1" w:author=" " w:initials=""><w:p><w:r><w:t>No initials.</w:t></w:r></w:p></w:comment>',
    ]))
    comment_body = (p(runs='<w:commentRangeStart w:id="0"/>' + r("commented") + '<w:commentRangeEnd w:id="0"/>'
                         '<w:r><w:commentReference w:id="0"/></w:r>' + r(" more")
                         + '<w:r><w:commentReference w:id="1"/></w:r>'))
    out["f-comments-default.docx"] = docx(comment_body, parts={"word/comments.xml": comments},
                                          doc_rels=[("rIdC", "comments", "comments.xml", False)])
    out["f-comments-mapped.docx"] = docx(comment_body, parts={
        "word/comments.xml": comments, "mammoth/style-map": "comment-reference => sup"},
        doc_rels=[("rIdC", "comments", "comments.xml", False)])
    out["f-comments-missing.docx"] = docx(
        p(runs='<w:r><w:commentReference w:id="9"/></w:r>'), parts={"mammoth/style-map": "comment-reference => sup"})

    out["f-sdt.docx"] = docx(
        '<w:sdt><w:sdtPr><w:alias w:val="x"/></w:sdtPr><w:sdtContent>' + p("inside block sdt") + '</w:sdtContent></w:sdt>'
        + p(runs='<w:sdt><w:sdtContent>' + r("inline sdt") + '</w:sdtContent></w:sdt>')
        + p(runs='<w:sdt><w:sdtPr><w14:checkbox><w14:checked w14:val="1"/></w14:checkbox></w:sdtPr>'
                 '<w:sdtContent><w:r><w:t>☒</w:t></w:r></w:sdtContent></w:sdt>' + r(" checked box"))
        + p(runs='<w:sdt><w:sdtPr><w14:checkbox><w14:checked w14:val="0"/></w14:checkbox></w:sdtPr>'
                 '<w:sdtContent><w:r><w:t>☐</w:t></w:r><w:r><w:t>second</w:t></w:r></w:sdtContent></w:sdt>' + r(" unchecked"))
        + p(runs='<w:sdt><w:sdtPr><w14:checkbox/></w:sdtPr><w:sdtContent></w:sdtContent></w:sdt>' + r(" empty box"))
        + p(runs='<w:sdt><w:sdtPr><w14:checkbox><w14:checked/></w14:checkbox></w:sdtPr>'
                 '<w:sdtContent><w:r><w:t></w:t></w:r><w:r><w:t>x</w:t></w:r></w:sdtContent></w:sdt>'))
    out["f-alternate.docx"] = docx(
        p(runs='<mc:AlternateContent><mc:Choice Requires="wps">' + r("choice") + '</mc:Choice>'
               '<mc:Fallback>' + r("fallback") + '</mc:Fallback></mc:AlternateContent>')
        + p(runs='<mc:AlternateContent><mc:Choice Requires="wps">' + r("only choice") + '</mc:Choice></mc:AlternateContent>')
        + p(runs='<mc:AlternateContent><mc:Fallback><mc:AlternateContent><mc:Fallback>' + r("nested fallback")
               + '</mc:Fallback></mc:AlternateContent></mc:Fallback></mc:AlternateContent>'))
    out["f-revisions.docx"] = docx(
        p(runs='<w:ins w:id="1">' + r("inserted") + '</w:ins><w:del w:id="2"><w:r><w:delText>deleted</w:delText></w:r></w:del>')
        + p(runs=r("deleted paragraph mark"), ppr="<w:rPr><w:del/></w:rPr>") + p("joins this one")
        + p(runs=r("deleted at the end"), ppr="<w:rPr><w:del/></w:rPr>"))
    out["f-textbox.docx"] = docx(
        p(runs=r("before box") + '<w:r><w:pict><v:shape><v:textbox><w:txbxContent>' + p("text box content")
               + '</w:txbxContent></v:textbox></v:shape></w:pict></w:r>' + r(" after box"))
        + p(runs='<w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing>choice drawing</w:drawing></mc:Choice>'
               '<mc:Fallback><w:pict><v:rect><v:textbox><w:txbxContent>' + p("fallback box")
               + '</w:txbxContent></v:textbox></v:rect></w:pict></mc:Fallback></mc:AlternateContent></w:r>')
        + '<w:pict><v:roundrect><v:textbox><w:txbxContent>' + p("pict at body level is lost")
        + '</w:txbxContent></v:textbox></v:roundrect></w:pict>'
        + '<w:tbl><w:tr><w:tc><w:pict><v:group><v:textbox><w:txbxContent>' + p("pict in cell")
        + '</w:txbxContent></v:textbox></v:group></w:pict>' + p("cell text") + '</w:tc></w:tr></w:tbl>')
    out["f-smarttag.docx"] = docx(
        p(runs='<w:smartTag w:uri="x" w:element="place">' + r("smart") + '</w:smartTag>'
               '<w:customXml w:element="c">' + r("custom xml is dropped") + '</w:customXml>'
               '<w:object>' + r("object") + '</w:object>'))
    out["f-empty-body.docx"] = docx("")
    out["f-no-body.docx"] = docx("", document_xml=part_xml("w:document", "<w:notbody/>"))
    out["f-no-styles.docx"] = docx(p("no styles part", "Heading1") + p(runs=r("x", '<w:rStyle w:val="Strong"/>')),
                                   styles=False)
    strict = ('xmlns:w="http://purl.oclc.org/ooxml/wordprocessingml/main" '
              'xmlns:r="http://purl.oclc.org/ooxml/officeDocument/relationships"')
    out["f-strict.docx"] = docx("", document_xml=part_xml(
        "w:document", "<w:body>" + p("strict namespace", "Heading1") + p(runs=r("bold", "<w:b/>")) + "</w:body>", ns=strict))
    out["f-prefixes.docx"] = docx("", document_xml=(
        f'<ns0:document xmlns:ns0="{W_NS}"><ns0:body><ns0:p><ns0:r><ns0:t>other prefix</ns0:t></ns0:r></ns0:p>'
        f'<p xmlns="{W_NS}"><r><t>default namespace</t></r></p>'
        f'<w:p xmlns:w="urn:not-word"><w:r><w:t>wrong namespace</w:t></w:r></w:p>'
        f'<x:p xmlns:x="{W_NS}" xmlns:w="urn:nope"><x:pPr><x:pStyle x:val="Heading1"/></x:pPr><x:r><x:t>x prefix heading</x:t></x:r></x:p>'
        f'</ns0:body></ns0:document>'))
    out["f-undeclared-prefix.docx"] = docx("", document_xml=(
        '<w:document><w:body><w:p><w:r><w:t>undeclared w prefix</w:t></w:r></w:p></w:body></w:document>'))
    out["f-part-paths.docx"] = docx(
        p("main part elsewhere", "Heading1"),
        package_rels=[("rId1", "officeDocument", "/custom/main.xml", False)],
        parts={"word/document.xml": None, "custom/main.xml": document(p("main part elsewhere", "Heading1")),
               "custom/_rels/main.xml.rels": rels([("rS", "styles", "../word/styles.xml", False),
                                                   ("rS2", "styles", "/word/styles.xml", False)])})
    out["f-part-fallback.docx"] = docx(
        p("fallback to word/document.xml"), package_rels=[("rId1", "officeDocument", "missing.xml", False)])
    out["f-part-missing.docx"] = docx(
        p("x"), package_rels=[("rId1", "officeDocument", "missing.xml", False)], parts={"word/document.xml": None})
    out["f-rels-no-target.docx"] = docx(
        p("x"), package_rels=[("rId1", "officeDocument", "word/document.xml", False)],
        parts={"_rels/.rels": (f'<Relationships xmlns="{REL_NS}"><Relationship Id="rId0" '
                               f'Type="{RT}officeDocument"/><Relationship Id="rId1" Type="{RT}officeDocument" '
                               f'Target="word/document.xml"/></Relationships>')})
    out["f-rels-root-text.docx"] = docx(p("x"), parts={"_rels/.rels": "   "})
    out["f-root-alternate.docx"] = docx(p("x"), parts={"word/styles.xml": (
        f'<mc:AlternateContent xmlns:mc="{MC_NS}"><mc:Fallback></mc:Fallback></mc:AlternateContent>')})
    out["f-root-alternate-text.docx"] = docx(p("x"), parts={"word/styles.xml": (
        f'<mc:AlternateContent xmlns:mc="{MC_NS}"><mc:Fallback> <w:styles xmlns:w="{W_NS}"/></mc:Fallback></mc:AlternateContent>')})
    out["f-override-no-partname.docx"] = docx(p("x"), parts={"[Content_Types].xml": (
        f'<Types xmlns="{CT_NS}"><Override ContentType="x"/></Types>')})
    out["f-styles-quirks.docx"] = docx(
        p("by duplicate id", "Normal") + p("type-less style", "Typeless") + p("styled empty name", "NoName")
        + p(runs=r("char style used as para style") , style="Strong"),
        styles=part_xml("w:styles",
                        '<w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Heading 1"/></w:style>'
                        '<w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style>'
                        '<w:style w:styleId="Typeless"><w:name w:val="Heading 2"/></w:style>'
                        '<w:style w:type="paragraph" w:styleId="NoName"/>'
                        '<w:style w:type="character" w:styleId="Strong"><w:name w:val="Heading 3"/></w:style>'))
    return out


# ---------------------------------------------------------------------------
# Pictures
# ---------------------------------------------------------------------------

def png(width, height, rows, color_type=2):
    """`rows(y)` returns the raw bytes of row y (no filter byte)."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    raw = b"".join(b"\0" + rows(y) for y in range(height))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, color_type, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def gradient_png(width, height):
    def row(y):
        return bytes(v for x in range(width) for v in ((x * 255) // max(1, width - 1), (y * 255) // max(1, height - 1), 128))
    return png(width, height, row)


def flat_png(width, height, rgb=(200, 30, 60)):
    line = bytes(rgb) * width
    return png(width, height, lambda y: line)


def noise_png(width, height, seed):
    rnd = random.Random(seed)
    tile = [bytes(rnd.getrandbits(8) for _ in range(width * 3)) for _ in range(8)]
    return png(width, height, lambda y: tile[y % 8])


def rgba_png(width, height):
    def row(y):
        return bytes(v for x in range(width) for v in (x % 256, y % 256, 90, (x + y) % 256))
    return png(width, height, row, color_type=6)


def vips_convert(data, suffix, tmp):
    src = os.path.join(tmp, "in.png")
    dst = os.path.join(tmp, "out" + suffix)
    open(src, "wb").write(data)
    subprocess.run(["vips", "copy", src, dst], check=True, capture_output=True)
    return open(dst, "rb").read()


def with_exif_orientation(jpeg, orientation):
    """Insert an APP1 Exif segment holding only Orientation after SOI."""
    tiff = b"MM\x00\x2a\x00\x00\x00\x08" + struct.pack(">H", 1) + struct.pack(">HHIHH", 0x0112, 3, 1, orientation, 0) \
        + b"\x00\x00\x00\x00"
    payload = b"Exif\x00\x00" + tiff
    app1 = b"\xff\xe1" + struct.pack(">H", len(payload) + 2) + payload
    assert jpeg[:2] == b"\xff\xd8"
    return jpeg[:2] + app1 + jpeg[2:]


EMF = b"\x01\x00\x00\x00\x6c\x00\x00\x00" + b"\x00" * 32 + b" EMF" + b"\x00" * 60


def drawing(rid, descr=None, title=None, anchor=False, hlink=None, link=False):
    attrs = (f' descr="{descr}"' if descr is not None else "") + (f' title="{title}"' if title is not None else "")
    h = f'<a:hlinkClick r:id="{hlink}"/>' if hlink else ""
    blip = f'<a:blip r:link="{rid}"/>' if link else (f'<a:blip r:embed="{rid}"/>' if rid else "<a:blip/>")
    tag = "wp:anchor" if anchor else "wp:inline"
    return (f'<w:r><w:drawing><{tag}><wp:extent cx="100" cy="100"/><wp:docPr id="1" name="Picture"{attrs}>{h}</wp:docPr>'
            f'<a:graphic><a:graphicData uri="{PIC_NS}"><pic:pic><pic:nvPicPr/><pic:blipFill>{blip}</pic:blipFill>'
            f'</pic:pic></a:graphicData></a:graphic></{tag}></w:drawing></w:r>')


def image_docs():
    out = {}
    tmp = tempfile.mkdtemp()
    try:
        small = gradient_png(64, 40)
        jpeg = vips_convert(gradient_png(300, 200), ".jpg", tmp)
        jpeg_exif = with_exif_orientation(jpeg, 6)
        gif = vips_convert(flat_png(30, 20), ".gif", tmp)
        media = lambda rid, name: (rid, "image", "media/" + name, False)
        out["i-png.docx"] = docx(
            p(runs=drawing("rImg1", descr="a gradient")) + p(runs=r("text ") + drawing("rImg1", descr="  ", title="titled")
                                                              + r(" after")),
            doc_rels=[media("rImg1", "g.png")], parts={"word/media/g.png": small})
        out["i-jpeg-exif.docx"] = docx(
            p(runs=drawing("rJ", anchor=True, descr="rotated")) + p(runs=drawing("rJ0")),
            doc_rels=[media("rJ", "photo.jpeg"), media("rJ0", "plain.jpg")],
            parts={"word/media/photo.jpeg": jpeg_exif, "word/media/plain.jpg": jpeg})
        out["i-formats.docx"] = docx(
            p(runs=drawing("rG")) + p(runs=drawing("rA", descr="alpha")) + p(runs=drawing("rT", descr="tiny"))
            + p(runs=drawing("rW")) + p(runs=drawing("rBmpName")) + p(runs=drawing("rUpper")),
            doc_rels=[media("rG", "f.gif"), media("rA", "a.png"), media("rT", "t.png"), media("rW", "wide.png"),
                      media("rBmpName", "x.bmp"), media("rUpper", "U.PNG")],
            parts={"word/media/f.gif": gif, "word/media/a.png": rgba_png(50, 40), "word/media/t.png": flat_png(1, 1),
                   "word/media/wide.png": flat_png(4000, 100, (10, 120, 200)), "word/media/x.bmp": small,
                   "word/media/U.PNG": small})
        out["i-24mpx.docx"] = docx(
            p(runs=drawing("rOk", descr="exactly 24 Mpx")) + p(runs=drawing("rBig", descr="one column more")),
            doc_rels=[media("rOk", "ok.png"), media("rBig", "big.png")],
            parts={"word/media/ok.png": flat_png(6000, 4000), "word/media/big.png": flat_png(6001, 4000)})
        out["i-unreadable.docx"] = docx(
            p(runs=drawing("rE", descr="emf")) + p(runs=drawing("rMissingFile", descr="gone"))
            + p(runs=drawing("rLink", link=True, descr="linked")) + p(runs=drawing(None)) + p(runs=drawing("rTxt"))
            + p(runs='<w:r><w:pict><v:shape><v:imagedata r:id="rV" o:title="vml title"/></v:shape></w:pict></w:r>'
                    '<w:r><w:pict><v:shape><v:imagedata o:title="no rid"/></v:shape></w:pict></w:r>' + r(" vml")),
            doc_rels=[media("rE", "e.emf"), media("rMissingFile", "nothere.png"),
                      ("rLink", "image", "file:///etc/passwd", True), media("rTxt", "t.txt"), media("rV", "v.png")],
            parts={"word/media/e.emf": EMF, "word/media/t.txt": b"not an image", "word/media/v.png": small})
        noisy = {f"word/media/n{i}.png": noise_png(1280, 1280, i) for i in range(6)}
        out["i-budget.docx"] = docx(
            "".join(p(runs=r(f"image {i} ") + drawing(f"rN{i}", descr=f"noise {i}" if i % 2 else None)) for i in range(6))
            + p(runs=drawing("rE", descr="emf after budget") + drawing("rMissingFile")) + p(runs=drawing("rS")),
            doc_rels=[media(f"rN{i}", f"n{i}.png") for i in range(6)]
            + [media("rE", "e.emf"), media("rMissingFile", "nothere.png"), media("rS", "s.png")],
            parts={**noisy, "word/media/e.emf": EMF, "word/media/s.png": small})
        out["i-hlink.docx"] = docx(
            p(runs=drawing("rI", hlink="rH")) + p(runs=drawing("rMissingFile", hlink="rNoSuchRel")),
            doc_rels=[media("rI", "i.png"), ("rH", "hyperlink", "http://example.com/img", True),
                      media("rMissingFile", "nothere.png")],
            parts={"word/media/i.png": small})
        out["i-hlink-missing.docx"] = docx(
            p(runs=drawing("rI", hlink="rNoSuchRel")), doc_rels=[media("rI", "i.png")], parts={"word/media/i.png": small})
        out["i-missing-rel.docx"] = docx(p(runs=drawing("rNoSuchRel")))
        out["i-twice.docx"] = docx(
            p(runs=drawing("rI", descr="first") + '<w:r><w:footnoteReference w:id="1"/></w:r>'
              + drawing("rI") + '<w:r><w:footnoteReference w:id="1"/></w:r>'),
            doc_rels=[media("rI", "i.png"), ("rFn", "footnotes", "footnotes.xml", False)],
            parts={"word/media/i.png": small, "word/footnotes.xml": part_xml(
                "w:footnotes", '<w:footnote w:id="1"><w:p>' + drawing("rFI", descr="in note") + "</w:p></w:footnote>"),
                "word/_rels/footnotes.xml.rels": rels([media("rFI", "i.png")])})
        out["i-ignored.docx"] = docx(
            p(runs=drawing("rI", descr="mapped away") + '<w:r><w:footnoteReference w:id="1"/></w:r>', style="Title")
            + p(runs=drawing("rI", descr="kept")),
            doc_rels=[media("rI", "i.png"), ("rFn", "footnotes", "footnotes.xml", False)],
            parts={"word/media/i.png": small, "mammoth/style-map": "p[style-name='Title'] => !",
                   "word/footnotes.xml": part_xml("w:footnotes", '<w:footnote w:id="1">' + p("note") + "</w:footnote>")})
        out["i-content-types.docx"] = docx(
            p(runs=drawing("rI")), doc_rels=[media("rI", "i.weird")],
            parts={"word/media/i.weird": small, "[Content_Types].xml": (
                f'<Types xmlns="{CT_NS}"><Default Extension="weird" ContentType="image/x-weird"/>'
                f'<Override PartName="/word/document.xml" ContentType="x"/></Types>')})
    finally:
        shutil.rmtree(tmp)
    return out


def size_docs():
    out = {}
    # MAX_HTML_BYTES is checked on `body.length`: UTF-16 code units.
    limit = 4 * 1024 * 1024
    one_para = lambda text: docx("", styles=False, document_xml=(
        f'<w:document xmlns:w="{W_NS}"><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>'))
    out["f-html-at-limit.docx"] = one_para("a" * (limit - 7))
    out["f-html-over-limit.docx"] = one_para("a" * (limit - 6))
    out["f-html-emoji-over.docx"] = one_para("\U0001F600" * ((limit - 6) // 2))
    out["f-html-escapes-over.docx"] = one_para("&amp;" * ((limit - 7) // 5 + 1))
    rows = "".join(f'<w:tr><w:tc>{p(f"cell {i}")}</w:tc><w:tc>{p(runs=r(f"bold {i}", "<w:b/>"))}</w:tc></w:tr>'
                   for i in range(2000))
    paras = "".join(p(f"Paragraph {i} " + "lorem ipsum dolor sit amet " * 5, "Heading2" if i % 50 == 0 else None)
                    + (li(f"item {i}", 1, i % 3) if i % 7 == 0 else "") for i in range(20000))
    out["f-large.docx"] = docx(paras + "<w:tbl>" + rows + "</w:tbl>", numbering=True)
    return out


# ---------------------------------------------------------------------------
# Embedded style maps
# ---------------------------------------------------------------------------

def style_map_docs():
    out = {}
    rich_map = "\n".join([
        "# a comment, then a blank line",
        "",
        "p[style-name='Title'] => h1.title.big:fresh",
        "p.Quote => blockquote > p:fresh",
        "r[style-name='Emphasis'] => em.emph",
        "b => span.bold",
        "i => !",
        "u => u",
        "strike => del",
        "small-caps => span[data-x='sc']",
        "all-caps => span.ac",
        "highlight[color='yellow'] => mark",
        "highlight => span.hl",
        "table.TableGrid => table.grid",
        "p:unordered-list(1) => ul.custom > li:fresh",
        "p[style-name^='Head'] => h4:fresh",
        "br[type='page'] => hr",
        "br[type='line'] => br.line",
        "p.Code => pre:separator('\\n')",
        r"p.Escaped => div[\1='one'][x='two'][\\b='three']",
        "this line is not a mapping",
        "p[style-name='a' => b",
        "p =>",
        "r[style-name='Strong'] => strong.s",
    ])
    styles = part_xml("w:styles", "".join([
        STYLES[STYLES.index("<w:styles") + len(f"<w:styles {NS_DECL}>"):STYLES.rindex("</w:styles>")],
        '<w:style w:type="paragraph" w:styleId="Code"><w:name w:val="Code"/></w:style>',
        '<w:style w:type="paragraph" w:styleId="Escaped"><w:name w:val="Escaped"/></w:style>',
        '<w:style w:type="paragraph" w:styleId="HeadLike"><w:name w:val="HeadLike thing"/></w:style>',
    ]))
    body = (p("A title", "Title") + p("quoted", "Quote") + p("headlike", "HeadLike")
            + p(runs=r("emph", '<w:rStyle w:val="Emphasis"/>') + r(" b", "<w:b/>") + r(" i", "<w:i/>")
                + r(" u", '<w:u w:val="single"/>') + r(" s", "<w:strike/>") + r(" sc", "<w:smallCaps/>")
                + r(" ac", "<w:caps/>") + r(" hy", '<w:highlight w:val="yellow"/>') + r(" hg", '<w:highlight w:val="green"/>')
                + r(" st", '<w:rStyle w:val="Strong"/>'))
            + p(runs='<w:r><w:t>line</w:t><w:br/><w:t>page</w:t><w:br w:type="page"/><w:t>end</w:t></w:r>')
            + p("code 1", "Code") + p("code 2", "Code") + p("code 3", "Code") + p("between")
            + p("escaped attrs", "Escaped")
            + '<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/></w:tblPr><w:tr><w:tc>' + p("grid") + '</w:tc></w:tr></w:tbl>'
            + li("custom bullet", 1, 0) + li("custom bullet 2", 1, 0))
    out["f-stylemap.docx"] = docx(body, styles=styles, numbering=True, parts={"mammoth/style-map": rich_map})
    out["f-stylemap-crlf-bom.docx"] = docx(
        p("A title", "Title"), parts={"mammoth/style-map": b"\xef\xbb\xbf  p[style-name='Title'] => h2.x  \r\n#c\r\n"})
    out["f-stylemap-br-unknown.docx"] = docx(p("x"), parts={"mammoth/style-map": "br[type='other'] => hr"})
    out["f-stylemap-br-unknown-empty.docx"] = docx("", parts={"mammoth/style-map": "br[type='other'] => hr"})
    out["f-stylemap-br-unknown-late.docx"] = docx(
        p("x", "Heading1"), parts={"mammoth/style-map": "p.Heading1 => h2\nbr[type='other'] => hr"})
    out["f-stylemap-undefined-path.docx"] = docx(
        p("x", "Heading1"), parts={"mammoth/style-map": "p.Heading1 => h1 > x[foo"})
    out["f-stylemap-undefined-path-unused.docx"] = docx(
        p("x", "Heading2"), parts={"mammoth/style-map": "p.Heading1 => h1 > x[foo"})
    out["f-stylemap-unicode.docx"] = docx(
        p("über", "Ueber") + p("strasse", "Strasse"),
        styles=part_xml("w:styles", '<w:style w:type="paragraph" w:styleId="Ueber"><w:name w:val="über"/></w:style>'
                                    '<w:style w:type="paragraph" w:styleId="Strasse"><w:name w:val="straße"/></w:style>'),
        parts={"mammoth/style-map": "p[style-name='ÜBER'] => h2\np[style-name='STRASSE'] => h3"})
    out["f-stylemap-lists.docx"] = docx(
        li("a", 2, 0) + li("b", 2, 1) + li("c", 1, 0),
        numbering=True,
        parts={"mammoth/style-map": "p:ordered-list(01) => ol.first > li:fresh\np:ordered-list(2) => div > p\n"
                                    "r:ordered-list(1) => span"})
    return out


# ---------------------------------------------------------------------------
# h-*: hostile archives
# ---------------------------------------------------------------------------

def basic_members(body="<w:p><w:r><w:t>hostile</w:t></w:r></w:p>", **kw):
    return [Member("[Content_Types].xml", content_types(), **kw.get("ct", {})),
            Member("_rels/.rels", rels([("rId1", "officeDocument", "word/document.xml", False)])),
            Member("word/document.xml", document(body), **kw.get("doc", {}))]


def hostile():
    out = {}
    out["h-not-a-zip.docx"] = b"this is not a docx at all, only text.\n"
    out["h-empty.docx"] = b""
    out["h-short.docx"] = b"PK\x03\x04" + b"\0" * 17
    out["h-ole2.doc"] = b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1" + b"\0" * 504
    out["h-oversized.docx"] = docx(p("big"), parts={"word/media/pad.bin": os.urandom(0) or random.Random(1).randbytes(11 * 1024 * 1024)},
                                   members_kw={"word/media/pad.bin": {}}, method=0)
    big = b"\0" * (50 * 1024 * 1024)
    out["h-bomb-honest.docx"] = build_zip(basic_members() + [Member("word/bomb.xml", big)])
    out["h-bomb-lying.docx"] = build_zip(basic_members() + [Member("word/bomb.xml", big, declared=100)])
    out["h-bomb-sum.docx"] = build_zip(basic_members() + [Member(f"word/b{i}.bin", b"\0" * (30 * 1024 * 1024)) for i in range(3)])
    out["h-zip64-count.docx"] = build_zip(basic_members(), eocd_fields={"on_disk": 0xFFFF, "total": 0xFFFF})
    out["h-zip64-central.docx"] = build_zip(basic_members(doc={"declared": 0xFFFFFFFF}))
    out["h-zip64-disk.docx"] = build_zip(basic_members(), eocd_fields={"disk": 0xFFFF})
    out["h-too-many.docx"] = build_zip(basic_members() + [Member(f"x/{i}", b"") for i in range(4094)])
    out["h-max-entries.docx"] = build_zip(basic_members() + [Member(f"x/{i}", b"", method=0) for i in range(4093)])
    out["h-method12.docx"] = build_zip(basic_members() + [Member("x.bin", b"abc", method=12, data=b"abc")])
    out["h-stored-mix.docx"] = build_zip(
        [Member(m.name, m.body, method=0 if i % 2 else 8) for i, m in enumerate(basic_members())])
    trunc = deflate_raw(document("<w:p><w:r><w:t>truncated</w:t></w:r></w:p>").encode())
    out["h-truncated-deflate.docx"] = build_zip(basic_members()[:2] + [
        Member("word/document.xml", document("<w:p><w:r><w:t>truncated</w:t></w:r></w:p>"), data=trunc[:-3])])
    out["h-garbage-deflate.docx"] = build_zip(basic_members()[:2] + [
        Member("word/document.xml", b"x" * 40, data=b"\x07\x00garbage")])
    out["h-trailing-deflate.docx"] = build_zip(basic_members()[:2] + [
        Member("word/document.xml", document("<w:p><w:r><w:t>trailing</w:t></w:r></w:p>"),
               data=deflate_raw(document("<w:p><w:r><w:t>trailing</w:t></w:r></w:p>").encode()) + b"junk")])
    out["h-data-descriptor.docx"] = build_zip(
        [Member(m.name, m.body, descriptor=True) for m in basic_members("<w:p><w:r><w:t>descriptors</w:t></w:r></w:p>")])
    out["h-duplicate-names.docx"] = build_zip(
        basic_members("<w:p><w:r><w:t>first copy</w:t></w:r></w:p>")
        + [Member("word/document.xml", document("<w:p><w:r><w:t>second copy wins</w:t></w:r></w:p>"))])
    out["h-extra-central.docx"] = build_zip(
        basic_members("<w:p><w:r><w:t>declared copy</w:t></w:r></w:p>"),
        extra_central=[Member("word/document.xml", document("<w:p><w:r><w:t>unverified copy</w:t></w:r></w:p>"))])
    out["iso-hang-extra-central-bomb.docx"] = build_zip(
        basic_members(), extra_central=[Member("word/document.xml", b"<" * (60 * 1024 * 1024))])
    out["h-extra-central-method.docx"] = build_zip(basic_members(), extra_central=[Member("z", b"abc", method=12, data=b"abc")])
    out["h-prepended.docx"] = build_zip(basic_members(), prepend=b"MZ" + b"\0" * 64)
    out["h-gap-before-trailer.docx"] = build_zip(basic_members(), between=b"\0" * 16)
    out["h-comment.docx"] = build_zip(basic_members(), comment=b"a legal comment")
    out["h-comment-fake-eocd.docx"] = build_zip(basic_members(), comment=b"xxPK\x05\x06" + b"\0" * 10)
    out["h-comment-overflow.docx"] = build_zip(basic_members(), comment=b"short", comment_len=200)
    out["h-encrypted.docx"] = build_zip(basic_members() + [Member("x.bin", b"secret", flags=1)])
    out["h-utf8-flag-names.docx"] = build_zip(basic_members() + [Member("word/médïa/ü.bin", b"x", flags=0x800)])
    upath = struct.pack("<BI", 1, zlib.crc32(b"word/other.xml")) + b"word/document.xml"
    ms = basic_members("<w:p><w:r><w:t>reached through the Unicode Path field</w:t></w:r></w:p>")
    ms[2] = Member("word/other.xml", ms[2].body, central_extra=struct.pack("<HH", 0x7075, len(upath)) + upath)
    out["h-unicode-path.docx"] = build_zip(ms)
    bad = basic_members()
    bad[2] = Member("word/document.xml", bad[2].body, central_extra=struct.pack("<HH", 0x7075, 3) + b"\x01ab")
    out["h-unicode-path-short.docx"] = build_zip(bad)
    out["h-declared-zero.docx"] = build_zip(basic_members(doc={"declared": 0}))
    out["h-size-mismatch.docx"] = build_zip(basic_members(doc={"declared": 1}))
    ms = basic_members()
    out["h-stored-size-mismatch.docx"] = build_zip(ms[:2] + [Member("word/document.xml", ms[2].body, method=0,
                                                                      declared=len(ms[2].body) + 5)])
    out["h-dir-entry.docx"] = build_zip(basic_members() + [Member("word/media/", b"", method=0),
                                                          Member("word/styles.xml", b"<x/>", external=0x10)])
    out["h-unix-dir-mode.docx"] = build_zip(basic_members() + [Member("x.bin", b"abc", made_by=0x0314,
                                                                      external=0o40755 << 16)])
    out["h-unix-file-mode.docx"] = build_zip(basic_members() + [Member("x.bin", b"abc", made_by=0x0314,
                                                                       external=0o100644 << 16)])
    out["h-local-name-differs.docx"] = build_zip(basic_members()[:2] + [
        Member("word/central-name.xml", document("<w:p><w:r><w:t>local header name wins</w:t></w:r></w:p>"),
               local_name=b"word/document.xml")])
    out["h-name-normalised.docx"] = build_zip(basic_members()[:2] + [
        Member("./word//x/../document.xml", document("<w:p><w:r><w:t>resolved name</w:t></w:r></w:p>"))])
    out["h-backslash-name.docx"] = build_zip(basic_members()[:2] + [
        Member("word\\document.xml", document("<w:p><w:r><w:t>backslash</w:t></w:r></w:p>"))])
    out["h-local-offset-bad.docx"] = build_zip(basic_members())[:-22] + struct.pack(
        "<IHHHHIIH", 0x06054B50, 0, 0, 3, 3, 0, 0, 0)
    z = bytearray(build_zip(basic_members()))
    z[0:4] = b"PK\x03\x05"
    out["h-bad-first-signature.docx"] = bytes(z)
    out["h-trailer-missing.docx"] = build_zip(basic_members())[:-22]
    return out


def iso_hang():
    """JSZip loops forever on ZIP64 extensible data whose length walks back."""
    ms = basic_members()
    z = build_zip(ms, eocd_fields={"disk": 0xFFFF})
    # A ZIP64 end record with 44 + 6 bytes of "extensible data" whose length
    # field is -6 (0xFFFFFFFA): the reader steps back to where it started.
    rec = struct.pack("<IQHHIIQQQQ", 0x06064B50, 44 + 6, 45, 45, 0, 0, 3, 3, 0, 0) + struct.pack("<HI", 1, 0xFFFFFFFA)
    loc = struct.pack("<IIQI", 0x07064B50, 0, len(z) - 22, 1)
    out = {"iso-hang-zip64-loop.docx": z[:-22] + rec + loc + z[-22:]}
    # Quadratic scans in xmldom that never report an error: one lastIndexOf
    # over the part per distinct element name, one indexOf('?>') plus a regex
    # over everything before it per unterminated `<?`.
    names = "".join(f"<w:n{i}></w:n{i} >" for i in range(400_000))
    out["iso-hang-many-names.docx"] = docx("", document_xml=document(p("many names") + "<w:p>" + names + "</w:p>"))
    out["iso-hang-pi-flood.docx"] = docx("", document_xml=document("<w:p>" + "<?x" * 1_000_000 + r("pi flood") + "</w:p>"))
    return out


# ---------------------------------------------------------------------------
# x-*: the XML parser
# ---------------------------------------------------------------------------

def xml_docs():
    out = {}
    doc = lambda inner, prolog='<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\r\n': (
        prolog + f"<w:document {NS_DECL}><w:body>{inner}</w:body></w:document>")
    raw = lambda name, xml: docx("", document_xml=xml) if isinstance(xml, str) else docx(
        "", parts={"word/document.xml": xml})
    out["x-bom.docx"] = raw("x", b"\xef\xbb\xbf" + doc(p("bom")).encode())
    out["x-bom2.docx"] = raw("x", b"\xef\xbb\xbf\xef\xbb\xbf" + doc(p("two boms")).encode())
    out["x-bom3.docx"] = raw("x", b"\xef\xbb\xbf" * 3 + doc(p("three boms")).encode())
    out["x-utf16.docx"] = raw("x", doc(p("utf16")).encode("utf-16"))
    out["x-invalid-utf8.docx"] = raw("x", doc(p("bad \xff\xfe bytes")).encode("latin-1").replace(b"\xc3", b"\xc3"))
    out["x-invalid-utf8-2.docx"] = raw("x", doc(p("cut")).encode().replace(b"cut", b"c\xe2\x82u\xed\xa0\x80t\xf0\x9f"))
    out["x-doctype.docx"] = raw("x", doc(p("doctype"), '<?xml version="1.0"?><!DOCTYPE w:document>\n'))
    out["x-doctype-subset.docx"] = raw("x", doc(p("subset"), '<?xml version="1.0"?><!DOCTYPE d [ <!ENTITY e "x"> ]>\n'))
    out["x-doctype-entity-use.docx"] = raw("x", doc(p("&e;"), '<!DOCTYPE d [ <!ENTITY e "x"> ]>'))
    out["x-doctype-after-root.docx"] = raw("x", doc(p("x")) + "<!DOCTYPE late>")
    out["x-doctype-inside.docx"] = raw("x", doc("<!DOCTYPE inner>" + p("doctype inside the body")))
    out["x-pi.docx"] = raw("x", doc(p(runs="<?pi data?>" + r("a") + "<?pi2?>" + r("b"))))
    out["x-pi-in-text.docx"] = raw("x", doc(p(runs="<w:r><w:t>a<?pi?>b</w:t></w:r>")))
    out["x-pi-unterminated.docx"] = raw("x", doc(p(runs="<w:r><w:t>a</w:t></w:r><?oops")))
    out["x-pi-unterminated-text.docx"] = raw("x", doc(p(runs="<w:r><w:t>a<?oops</w:t></w:r>")))
    out["x-pi-unterminated-noprolog.docx"] = raw("x", doc(p(runs="<w:r><w:t>a<?oops</w:t></w:r>"), prolog=""))
    out["x-stray-end.docx"] = raw("x", doc(p(runs="</w:nothing>" + r("stray end tags") + "</w:r>")))
    out["x-bare-lt.docx"] = raw("x", doc(p(runs="<w:r><w:t>1 <!x 2 <!-y 3</w:t></w:r>")))
    out["x-bare-lt-space.docx"] = raw("x", doc(p(runs="<w:r><w:t>a < b</w:t></w:r>")))
    out["x-bare-amp.docx"] = raw("x", doc(p(runs="<w:r><w:t>AT&T &amp &#; & ;</w:t></w:r>")))
    out["x-entity-unknown.docx"] = raw("x", doc(p("&nbsp;")))
    out["x-entities-attr.docx"] = raw("x", doc(p(runs=drawing("rX", descr="line1&#10;line2\ttab\nnl &amp; &lt;&quot;")),
                                         ) if False else doc(p(runs='<w:hyperlink w:anchor="a&#10;b c&#9;d e\tf"><w:r><w:t>attr whitespace</w:t></w:r></w:hyperlink>')))
    out["x-dup-attr.docx"] = raw("x", doc('<w:p w:a="1" w:a="2"><w:r><w:t>x</w:t></w:r></w:p>'))
    out["x-dup-attr-ns.docx"] = raw("x", doc(
        f'<w:p xmlns:x="{W_NS}"><w:hyperlink w:anchor="one" x:anchor="two"><w:r><w:t>same name twice</w:t></w:r></w:hyperlink></w:p>'))
    out["x-unquoted.docx"] = raw("x", doc('<w:p w:a=1><w:r><w:t>x</w:t></w:r></w:p>'))
    out["x-no-space-attrs.docx"] = raw("x", doc('<w:p w:a="1"w:b="2"><w:r><w:t>x</w:t></w:r></w:p>'))
    out["x-unclosed-nested.docx"] = raw("x", doc('<w:p><w:r><w:t>a</w:t></w:r>' + p("b")))
    out["x-unclosed-last.docx"] = raw("x", doc(p("a") + '<w:p><w:r><w:t>b</w:t></w:r>'))
    out["x-case-mismatch.docx"] = raw("x", doc('<w:p><w:r><w:t>x</w:t></w:r></W:P>'))
    out["x-cdata-split.docx"] = raw("x", doc(p(runs="<w:r><w:t>a<![CDATA[b]]>c</w:t></w:r>")))
    out["x-comment-split.docx"] = raw("x", doc(p(runs="<w:r><w:t>a<!--x-->b</w:t></w:r>")))
    out["x-comment-only.docx"] = raw("x", doc(p(runs="<w:r><w:t><!--x-->b</w:t></w:r><!-- between -->" + r("c"))))
    out["x-cdata-unterminated.docx"] = raw("x", doc(p(runs="<w:r><w:t>x</w:t></w:r>")) + "<![CDATA[ never ends")
    out["x-newlines.docx"] = raw("x", doc(p(runs="<w:r><w:t>cr\rlf\ncrlf\r\nnel\u0085ls ps end</w:t></w:r>")))
    out["x-u0080.docx"] = raw("x", doc('<w:p\u0080><w:r\u0080 ><w:t>u0080 as space</w:t></w:r></w:p>'))
    out["x-self-close-space.docx"] = raw("x", doc('<w:p><w:r><w:t>x</w:t><w:br/ ></w:r></w:p>'))
    out["x-xhtml.docx"] = raw("x", doc(
        '<w:p><h:div xmlns:h="http://www.w3.org/1999/xhtml" checked>x</h:div>'
        '<div xmlns="http://www.w3.org/1999/xhtml"><input checked/><script>if (a < b) {}</script></div>'
        '<w:r><w:t>after xhtml</w:t></w:r></w:p>'))
    out["x-trailing-text.docx"] = raw("x", doc(p("x")) + "\n trailing text \n")
    out["x-leading-text.docx"] = raw("x", "leading junk " + doc(p("x")))
    out["x-two-roots.docx"] = raw("x", doc(p("x")) + "<w:document/>")
    out["x-empty-part.docx"] = raw("x", b"")
    out["x-whitespace-part.docx"] = raw("x", b"  \n ")
    out["x-comment-only-part.docx"] = raw("x", b"<!-- nothing -->")
    out["x-surrogates.docx"] = raw("x", doc(
        p(runs="<w:r><w:t>lone &#xD800; pair &#xD83D;&#xDE00; reversed &#xDE00;&#xD83D; big &#x110000; &#xFFFF; nul &#0;</w:t></w:r>")
        + p(runs="<w:r><w:t>&#xD83D;</w:t></w:r><w:r><w:t>&#xDE00;</w:t></w:r>")
        + p(runs="<w:r><w:t>&#xD83D;</w:t><w:t>&#xDE01;</w:t></w:r>")))
    out["x-numeric-quirks.docx"] = raw("x", doc(p(runs="<w:r><w:t>[&#x4x;][&#1x;][&#X41;][&#0x41;][&#xx41;][&#_1;][&#99999999999;][&#xFFFFFFFFFF;]</w:t></w:r>")))
    out["x-namespace-redeclared.docx"] = raw("x", (
        f'<w:document xmlns:w="{W_NS}"><w:body><w:p xmlns:w="urn:other"><w:r><w:t>redeclared</w:t></w:r></w:p>'
        f'<w:p><w:r><w:t>back</w:t></w:r></w:p></w:body></w:document>'))
    out["x-xml-prefix.docx"] = raw("x", doc(p(runs='<w:r><w:t xml:space="preserve" xml:lang="en"> xml attrs </w:t></w:r>')))
    out["x-unicode-names.docx"] = raw("x", doc('<w:p><w:r><w:t>x</w:t></w:r><w:é/><w:𝒳/></w:p>'))
    out["x-many-names.docx"] = raw("x", doc("<w:p>" + "".join(f"<w:n{i}></w:n{i} >" for i in range(3000))
                                       + r("3000 distinct names") + "</w:p>"))
    out["x-pi-many.docx"] = raw("x", doc("<w:p>" + "<?x" * 3000 + r("3000 unterminated PIs") + "</w:p>"))
    out["x-deep-ok.docx"] = raw("x", doc("<w:p>" + "<w:x>" * 1898 + "</w:x>" * 1898 + "</w:p>"))
    out["x-deep-fail.docx"] = raw("x", doc("<w:p>" + "<w:x>" * 1899 + "</w:x>" * 1899 + "</w:p>"))
    out["x-deep-tables-400.docx"] = raw("x", doc("<w:tbl><w:tr><w:tc>" * 400 + p("deep") + "</w:tc></w:tr></w:tbl>" * 400))
    out["x-deep-sdt-920.docx"] = raw("x", doc("<w:sdt><w:sdtContent>" * 920 + p("deep") + "</w:sdtContent></w:sdt>" * 920))
    out["x-deep-tables-ok.docx"] = raw("x", doc("<w:tbl><w:tr><w:tc>" * 40 + p("deep") + "</w:tc></w:tr></w:tbl>" * 40))
    out["x-deep-plain-2000.docx"] = raw("x", doc("<w:p>" + "<x>" * 2000 + "</x>" * 2000 + "</w:p>"))
    return out


# ---------------------------------------------------------------------------
# iso-*: Object.prototype members as names
# ---------------------------------------------------------------------------

def proto_docs():
    out = {}
    doc = lambda inner: f'<?xml version="1.0"?><w:document {NS_DECL}><w:body>{inner}</w:body></w:document>'
    quiet_styles = part_xml("w:styles", '<w:style w:type="paragraph" w:styleId="Normal"><w:name w:val="Normal"/></w:style>'
                                        '<w:style w:type="paragraph" w:styleId="Hidden"><w:name w:val="Hidden"/></w:style>')
    out["iso-proto-element-converted.docx"] = docx("", styles=quiet_styles, document_xml=doc(
        '<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr><w:r><w:t>x</w:t></w:r><constructor/></w:p>'))
    out["iso-proto-element-ignored.docx"] = docx("", styles=quiet_styles, parts={"mammoth/style-map": "p.Hidden => !"},
                                                 document_xml=doc(
        '<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr><w:r><w:t>shown</w:t></w:r></w:p>'
        '<w:p><w:pPr><w:pStyle w:val="Hidden"/></w:pPr><w:r><w:t>hidden</w:t></w:r><toString/></w:p>'))
    out["iso-proto-element-ignored-warned.docx"] = docx("", styles=quiet_styles, parts={"mammoth/style-map": "p.Hidden => !"},
                                                        document_xml=doc(
        '<w:p><w:pPr><w:pStyle w:val="Normal"/></w:pPr><w:r><w:t>shown</w:t></w:r><w:unknownThing/></w:p>'
        '<w:p><w:pPr><w:pStyle w:val="Hidden"/></w:pPr><w:r><w:t>hidden</w:t></w:r><toString/></w:p>'))
    out["iso-proto-element-two.docx"] = docx("", styles=quiet_styles, parts={"mammoth/style-map": "p.Hidden => !"},
                                             document_xml=doc(
        '<w:p><w:pPr><w:pStyle w:val="Hidden"/></w:pPr><constructor/><toString/></w:p>'))
    out["iso-proto-element-hasown.docx"] = docx("", document_xml=doc('<w:p><hasOwnProperty/></w:p>'))
    out["iso-proto-element-proto.docx"] = docx("", document_xml=doc('<w:p><__proto__/></w:p>'))
    out["iso-proto-element-in-table.docx"] = docx("", styles=quiet_styles, document_xml=doc(
        '<w:tbl><w:tr><constructor/></w:tr></w:tbl>'))
    out["iso-proto-styles.docx"] = docx(
        p("toString style", "toString") + p("constructor style", "constructor") + p("proto style", "__proto__")
        + p("valueOf style", "valueOf"),
        parts={"mammoth/style-map": "p[style-name='toString'] => h3\np[style-name='Object'] => h4\n"
                                    "p[style-name='valueOf'] => h5"})
    out["iso-proto-style-defs.docx"] = docx(
        p("defined toString", "toString"),
        styles=part_xml("w:styles", '<w:style w:type="paragraph" w:styleId="toString"><w:name w:val="Heading 1"/></w:style>'
                                    '<w:style w:type="constructor" w:styleId="x"><w:name w:val="x"/></w:style>'))
    out["iso-proto-pollution.docx"] = docx(
        p(runs='<w:r><w:t>line</w:t><w:br/><w:t>break</w:t></w:r>') + p("styled", "Heading1"),
        styles=part_xml("w:styles", '<w:style w:type="__proto__" w:styleId="w:type"><w:name w:val="polluted"/></w:style>'
                                    + STYLES[STYLES.index("<w:style "):STYLES.rindex("</w:styles>")]))
    out["iso-proto-numbering.docx"] = docx(
        li("numId constructor", "constructor", 0) + li("numId __proto__", "__proto__", 0) + li("ilvl toString", 1, "toString")
        + li("abstract toString", 8, 0),
        numbering=numbering_xml('<w:num w:numId="8"><w:abstractNumId w:val="toString"/></w:num>'))
    out["iso-proto-numbering-ok.docx"] = docx(
        li("numId constructor", "constructor", 0) + li("ilvl toString", 1, "toString")
        + p("pstyle toString", "toString", ppr='<w:numPr><w:numId w:val="1"/></w:numPr>'),
        numbering=numbering_xml('<w:abstractNum w:abstractNumId="__proto__"><w:lvl w:ilvl="0"><w:numFmt w:val="bullet"/>'
                                '<w:pStyle w:val="ProtoListStyle"/></w:lvl></w:abstractNum>'))
    out["iso-proto-rel-type.docx"] = docx(p("x"), doc_rels=[("rX", "constructor", "x", False)])
    out["iso-proto-rel-id.docx"] = docx(p(runs='<w:hyperlink r:id="toString"><w:r><w:t>to a function</w:t></w:r></w:hyperlink>'))
    out["iso-proto-rel-id-empty.docx"] = docx(p(runs='<w:hyperlink r:id="toString"></w:hyperlink>' + r("nothing written")))
    out["iso-proto-ct-hasown.docx"] = docx(
        p(runs=drawing("rI")), doc_rels=[("rI", "image", "media/i.png", False)],
        extra_ct=[("hasOwnProperty", "x/y")], parts={"word/media/i.png": flat_png(4, 4)})
    out["iso-proto-ct-override.docx"] = docx(
        p(runs=drawing("rI")), doc_rels=[("rI", "image", "media/constructor", False)],
        parts={"word/media/constructor": flat_png(4, 4)})
    out["iso-proto-attrs.docx"] = docx("", document_xml=doc(
        '<w:p __proto__="1" __proto__="2"><w:r><w:t>proto attrs</w:t></w:r></w:p>'))
    out["iso-proto-attr-hasown.docx"] = docx("", document_xml=doc(
        '<w:p hasOwnProperty="1" b="2"><w:r><w:t>x</w:t></w:r></w:p>'))
    out["iso-proto-closemap.docx"] = docx("", document_xml=doc(
        '<w:p><w:r><w:t>constructor never closed</w:t></w:r><constructor></w:p>'))
    out["iso-proto-ns.docx"] = docx("", document_xml=doc(
        '<w:p><constructor:x/><w:r><w:t>undeclared builtin prefix</w:t></w:r></w:p>'
        '<w:p xmlns:k="constructor"><k:r><k:t>namespace named constructor</k:t></k:r></w:p>'))
    out["iso-proto-comments.docx"] = docx(
        p(runs='<w:r><w:commentReference w:id="toString"/></w:r>'), parts={"mammoth/style-map": "comment-reference => sup"})
    out["iso-proto-bookmark.docx"] = docx(p(runs='<w:bookmarkStart w:name="constructor"/>' + r("x")))
    return out


# ---------------------------------------------------------------------------
# t-*: macOS textutil
# ---------------------------------------------------------------------------

TEXTUTIL_HTML = {
    "t-structure": """<html><head><meta charset="utf-8"><title>T</title></head><body>
<h1>Heading One</h1><h2>Heading Two</h2><h3>Heading Three</h3>
<p>Para with <b>bold</b>, <i>italic</i>, <u>under</u>, <s>strike</s>, x<sup>2</sup>, H<sub>2</sub>O and
<a href="http://example.com/?q=1&amp;r=2">a link</a>.</p>
<ul><li>one</li><li>two<ul><li>nested</li><li>nested two</li></ul></li><li>three</li></ul>
<ol><li>first</li><li>second<ol><li>sub</li></ol></li></ol>
<blockquote>quoted text</blockquote><pre>pre  formatted
  line two</pre>
<p>Special: &lt;tag&gt; &amp; “quotes” — dash … ellipsis ß ü 中文 😀</p></body></html>""",
    "t-tables": """<html><head><meta charset="utf-8"></head><body>
<table border="1"><tr><th>H1</th><th>H2</th><th>H3</th></tr>
<tr><td colspan="2">span two</td><td rowspan="2">rows</td></tr><tr><td>a</td><td>b</td></tr></table>
<p>between</p><table><tr><td><table border="1"><tr><td>nested</td></tr></table></td><td>outer</td></tr></table>
</body></html>""",
}

TEXTUTIL_RTF = r"""{\rtf1\ansi\deff0{\fonttbl{\f0 Helvetica;}{\f1 Symbol;}}
{\b Bold heading}\par
Normal text with {\i italic} and {\ul underline} and {\strike strike}.\par
{\field{\*\fldinst{HYPERLINK "http://example.com/rtf"}}{\fldrslt link text}}\par
Tab\tab separated\line new line\par
\page After page break\par
}"""


def textutil_docs():
    out = {}
    if not shutil.which("textutil"):
        return out
    tmp = tempfile.mkdtemp()
    try:
        img = os.path.join(tmp, "pic.png")
        open(img, "wb").write(gradient_png(120, 80))
        html = dict(TEXTUTIL_HTML)
        html["t-image"] = f'<html><body><p>Before image</p><p><img src="{img}" alt="pic"></p><p>After</p></body></html>'
        for name, source in html.items():
            src = os.path.join(tmp, name + ".html")
            open(src, "w").write(source)
            dst = os.path.join(tmp, name + ".docx")
            subprocess.run(["textutil", "-convert", "docx", src, "-output", dst], check=True, capture_output=True)
            out[name + ".docx"] = open(dst, "rb").read()
        src = os.path.join(tmp, "t-rtf.rtf")
        open(src, "w").write(TEXTUTIL_RTF)
        dst = os.path.join(tmp, "t-rtf.docx")
        subprocess.run(["textutil", "-convert", "docx", src, "-output", dst], check=True, capture_output=True)
        out["t-rtf.docx"] = open(dst, "rb").read()
    finally:
        shutil.rmtree(tmp)
    return out


def more_features():
    out = {}
    L = lambda rid, text, extra="": f'<w:hyperlink r:id="{rid}"{extra}><w:r><w:t>{text}</w:t></w:r></w:hyperlink>'
    links = [("rA", "hyperlink", "http://a.example/", True), ("rB", "hyperlink", "http://b.example/", True)]
    out["f-simplify.docx"] = docx(
        p(runs=L("rA", "a1") + L("rA", "a2") + L("rB", "b") + L("rA", "a3") + L("rA", "t", ' w:tgtFrame="x"'))
        + p(runs='<w:bookmarkStart w:id="1" w:name="only"/>') + p(runs='<w:r><w:t></w:t></w:r>')
        + p(runs=r("x", "<w:b/>") + r("", "<w:b/>") + r("y", "<w:b/>") + r("z", "<w:i/>") + r("w", "<w:b/><w:i/>"))
        + p(runs='<w:r><w:rPr><w:b/></w:rPr><w:tab/></w:r><w:r><w:rPr><w:b/></w:rPr><w:br/></w:r>')
        + p(runs='<w:hyperlink r:id="rA"><w:r><w:t>with</w:t><w:br/><w:tab/><w:t>breaks</w:t></w:r></w:hyperlink>')
        + p(runs='<w:hyperlink w:anchor=""><w:r><w:t>empty anchor</w:t></w:r></w:hyperlink>')
        + p(runs='<w:ins><w:hyperlink r:id="rA"><w:r><w:t>ins link</w:t></w:r></w:hyperlink></w:ins>'
                 '<w:moveTo><w:r><w:t>moved</w:t></w:r></w:moveTo><w:moveFrom><w:r><w:t>from</w:t></w:r></w:moveFrom>')
        + '<w:ins>' + p("inserted paragraph") + '</w:ins>'
        + p(runs=r("deleted with table after"), ppr="<w:rPr><w:del/></w:rPr>")
        + '<w:tbl><w:tr><w:tc>' + p("cell after deleted") + '</w:tc></w:tr></w:tbl>' + p("receives deleted")
        + '<w:p><w:r><w:t>outer</w:t></w:r><w:p><w:r><w:t>nested paragraph</w:t></w:r></w:p></w:p>'
        + p(runs='<w:r><w:t>run with</w:t><w:p><w:r><w:t>paragraph inside run</w:t></w:r></w:p></w:r>'),
        doc_rels=links)
    out["f-notes-edges.docx"] = docx(
        p(runs=r("a") + '<w:r><w:footnoteReference/></w:r>' + r("b") + '<w:r><w:footnoteReference w:id="5"/></w:r>'
               + '<w:r><w:endnoteReference w:id="5"/></w:r>'),
        parts={"word/footnotes.xml": part_xml("w:footnotes",
                                              '<w:footnote><w:p><w:r><w:t>no id</w:t></w:r></w:p></w:footnote>'
                                              '<w:footnote w:id="5"><w:p><w:r><w:t>first five</w:t></w:r></w:p></w:footnote>'
                                              '<w:footnote w:id="5"><w:p><w:r><w:t>second five</w:t></w:r></w:p></w:footnote>'
                                              '<w:footnote w:id="6"><w:pict><v:shape><v:textbox><w:txbxContent>'
                                              + p("lost extra") + '</w:txbxContent></v:textbox></v:shape></w:pict></w:footnote>'),
               "word/endnotes.xml": part_xml("w:endnotes", '<w:endnote w:id="5"><w:p><w:r><w:t>end five</w:t></w:r></w:p></w:endnote>')})
    out["f-endnote-missing-part.docx"] = docx(p(runs='<w:r><w:endnoteReference w:id="1"/></w:r>'))
    out["f-comments-twice.docx"] = docx(
        p(runs='<w:r><w:commentReference w:id="0"/></w:r>' + r(" x ") + '<w:r><w:commentReference w:id="0"/></w:r>'
               + '<w:r><w:footnoteReference w:id="1"/></w:r>'),
        parts={"mammoth/style-map": "comment-reference => sup",
               "word/comments.xml": part_xml("w:comments",
                                             '<w:comment w:id="0" w:initials=" ZZ "><w:p><w:r><w:t>c0 </w:t></w:r>'
                                             '<w:r><w:commentReference w:id="1"/></w:r></w:p></w:comment>'
                                             '<w:comment w:id="1"><w:p><w:r><w:t>c1</w:t></w:r></w:p></w:comment>'),
               "word/footnotes.xml": part_xml("w:footnotes", '<w:footnote w:id="1"><w:p><w:r><w:commentReference w:id="1"/>'
                                                             '</w:r></w:p></w:footnote>')})
    out["f-unicode-case.docx"] = docx(
        p("ff ligature", "Lig") + p("dotted", "Dot") + p("sigma", "Sig") + p("long s", "LongS"),
        styles=part_xml("w:styles",
                        '<w:style w:type="paragraph" w:styleId="Lig"><w:name w:val="ﬀ"/></w:style>'
                        '<w:style w:type="paragraph" w:styleId="Dot"><w:name w:val="İstanbul"/></w:style>'
                        '<w:style w:type="paragraph" w:styleId="Sig"><w:name w:val="ΟΔΟΣ"/></w:style>'
                        '<w:style w:type="paragraph" w:styleId="LongS"><w:name w:val="ſtyle"/></w:style>'),
        parts={"mammoth/style-map": "p[style-name='FF'] => h1\np[style-name='i̇stanbul'] => h2\n"
                                    "p[style-name^='οδος'] => h3\np[style-name='STYLE'] => h4"})
    out["f-stylemap-tokens.docx"] = docx(
        p("tabs", "Heading1") + p("quote", "Quote") + p("title", "Title") + p("body", "Body"),
        parts={"mammoth/style-map": "\n".join([
            "p.Heading1\t=>\th2.a\\.b",
            "p[style-name='Quote\\'s'] => h3",
            "p[style-name='Quote'] =>  h4",
            "p.Title=>h5",
            "p[style-name='Title'] => span:separator('\\n'):fresh",
            "p[style-name='Title'] => span:fresh:separator('|')",
            "p.Body => p.x.y.z[class='w'].v",
            "p.Body => q",
            "p[style-name='unterminated] => h6",
        ])})
    return out


def mammoth_test_data():
    """mammoth's own test documents (written by Word and LibreOffice), from
    the package sources copied out of the reference image, when present."""
    out = {}
    here = os.path.dirname(os.path.abspath(__file__))
    d = here
    while d != "/" and not os.path.isdir(os.path.join(d, "_scratch")):
        d = os.path.dirname(d)
    src = os.path.join(d, "_scratch/srv-media-a/ref-src/mammoth@1.12.1/node_modules/mammoth/test/test-data")
    if os.path.isdir(src):
        for name in sorted(os.listdir(src)):
            path = os.path.join(src, name)
            if name.endswith(".docx") or name.endswith(".zip"):
                out["m-" + name.replace(".zip", "-zip.docx")] = open(path, "rb").read()
            elif os.path.isdir(path):
                # Unpacked part trees: zipped as they are.
                members = []
                for root, _, files in sorted(os.walk(path)):
                    for f in sorted(files):
                        full = os.path.join(root, f)
                        members.append(Member(os.path.relpath(full, path), open(full, "rb").read()))
                out["m-dir-" + name + ".docx"] = build_zip(members)
    return out


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    out_dir = sys.argv[1]
    shutil.rmtree(out_dir, ignore_errors=True)
    os.makedirs(out_dir)
    random.seed(7)
    files = {}
    for family in (features, more_features, size_docs, style_map_docs, image_docs, hostile, iso_hang, xml_docs, proto_docs, textutil_docs,
                   mammoth_test_data):
        files.update(family())
    for name, data in sorted(files.items()):
        open(os.path.join(out_dir, name), "wb").write(data)
    print(f"{len(files)} files in {out_dir}")


if __name__ == "__main__":
    main()

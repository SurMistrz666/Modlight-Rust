//! tests/integration/docx.test.ts, plus the parity harness of
//! `tests/parity_media/docx_parity.py` (ignored: run on demand).

use flate2::Compression;
use flate2::write::DeflateEncoder;
use std::io::Write;

use bytes::Bytes;
use modlight_server::media::docx::{DocxRefusal, docx_to_html};
use modlight_server::media::zip_guard::ZipRefusal;

fn deflate_raw(data: &[u8]) -> Vec<u8> {
    let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn crc32(buf: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (n, slot) in table.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    let mut crc = 0xffff_ffffu32;
    for &b in buf {
        crc = table[((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    crc ^ 0xffff_ffff
}

/// Build a real .docx in memory.
///
/// A .docx is a ZIP holding a handful of OOXML parts. Only three are needed for
/// mammoth to read a document body, so this writes those rather than pulling in a
/// document-authoring dependency to test a converter.
fn build_docx(body_xml: &str, extra: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut members: Vec<(String, Vec<u8>)> = vec![
        (
            "[Content_Types].xml".into(),
            br#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#.to_vec(),
        ),
        (
            "_rels/.rels".into(),
            br#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#.to_vec(),
        ),
        (
            "word/document.xml".into(),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body_xml}</w:body></w:document>"#
            )
            .into_bytes(),
        ),
    ];
    members.extend(extra.iter().map(|(n, b)| ((*n).to_owned(), b.clone())));

    let mut locals = Vec::new();
    let mut centrals = Vec::new();
    let mut offset = 0u32;
    for (name, body) in &members {
        let data = deflate_raw(body);
        let name = name.as_bytes();
        let crc = crc32(body);

        let mut local = vec![0u8; 30];
        local[0..4].copy_from_slice(&0x0403_4b50u32.to_le_bytes());
        local[4..6].copy_from_slice(&20u16.to_le_bytes());
        local[8..10].copy_from_slice(&8u16.to_le_bytes());
        local[14..18].copy_from_slice(&crc.to_le_bytes());
        local[18..22].copy_from_slice(&(data.len() as u32).to_le_bytes());
        local[22..26].copy_from_slice(&(body.len() as u32).to_le_bytes());
        local[26..28].copy_from_slice(&(name.len() as u16).to_le_bytes());
        locals.extend_from_slice(&local);
        locals.extend_from_slice(name);
        locals.extend_from_slice(&data);

        let mut central = vec![0u8; 46];
        central[0..4].copy_from_slice(&0x0201_4b50u32.to_le_bytes());
        central[6..8].copy_from_slice(&20u16.to_le_bytes());
        central[10..12].copy_from_slice(&8u16.to_le_bytes());
        central[16..20].copy_from_slice(&crc.to_le_bytes());
        central[20..24].copy_from_slice(&(data.len() as u32).to_le_bytes());
        central[24..28].copy_from_slice(&(body.len() as u32).to_le_bytes());
        central[28..30].copy_from_slice(&(name.len() as u16).to_le_bytes());
        central[42..46].copy_from_slice(&offset.to_le_bytes());
        centrals.extend_from_slice(&central);
        centrals.extend_from_slice(name);

        offset += (30 + name.len() + data.len()) as u32;
    }
    let mut eocd = vec![0u8; 22];
    eocd[0..4].copy_from_slice(&0x0605_4b50u32.to_le_bytes());
    eocd[8..10].copy_from_slice(&(members.len() as u16).to_le_bytes());
    eocd[10..12].copy_from_slice(&(members.len() as u16).to_le_bytes());
    eocd[12..16].copy_from_slice(&(centrals.len() as u32).to_le_bytes());
    eocd[16..20].copy_from_slice(&(locals.len() as u32).to_le_bytes());
    let mut out = locals;
    out.extend_from_slice(&centrals);
    out.extend_from_slice(&eocd);
    out
}

fn para(text: &str, style: Option<&str>) -> String {
    let props = style.map(|s| format!(r#"<w:pPr><w:pStyle w:val="{s}"/></w:pPr>"#)).unwrap_or_default();
    format!("<w:p>{props}<w:r><w:t>{text}</w:t></w:r></w:p>")
}

async fn convert(docx: Vec<u8>) -> modlight_server::media::docx::DocxResult {
    docx_to_html(Bytes::from(docx)).await
}

// --- converting a document ---------------------------------------------------------------

#[tokio::test]
async fn turns_a_real_document_into_structured_html() {
    let docx = build_docx(&(para("A Title", Some("Heading1")) + &para("Some body text.", None)), &[]);
    let result = convert(docx).await.expect("converted");
    assert!(result.html.contains("Some body text."));
    assert!(result.html.contains("<h1>A Title</h1>"));
}

#[tokio::test]
async fn produces_a_self_contained_page_because_the_sandbox_can_load_nothing() {
    // The frame is an opaque origin under `default-src 'none'`: no stylesheet,
    // no font and no script of ours can reach it.
    let result = convert(build_docx(&para("hello", None), &[])).await.expect("converted");
    assert!(result.html.to_lowercase().starts_with("<!doctype html>"));
    assert!(result.html.contains("<style>"));
    let link = regex::Regex::new(r"(?i)<link[^>]+href").unwrap();
    assert!(!link.is_match(&result.html));
    assert!(!result.html.to_lowercase().contains("<script"));
}

#[tokio::test]
async fn follows_the_readers_colour_scheme_rather_than_flashing_white() {
    let result = convert(build_docx(&para("hello", None), &[])).await.expect("converted");
    assert!(result.html.contains("prefers-color-scheme: dark"));
}

#[tokio::test]
async fn escapes_text_that_would_otherwise_become_markup() {
    let result =
        convert(build_docx(&para("&lt;script&gt;alert(1)&lt;/script&gt;", None), &[])).await.expect("converted");
    // mammoth escapes, and the sandbox is what makes it not matter if it ever
    // stops - but the escaping should still be true today.
    assert!(!result.html.contains("<script>alert(1)"));
    assert!(result.html.contains("&lt;script&gt;"));
}

#[tokio::test]
async fn refuses_a_zip_bomb_before_mammoth_is_handed_the_buffer() {
    // Once JSZip has it, expansion is synchronous and no timeout can help.
    let bomb = build_docx(&para("x", None), &[("word/media/huge.bin", vec![0; 120 * 1024 * 1024])]);
    assert_eq!(convert(bomb).await, Err(DocxRefusal::Zip(ZipRefusal::TooBig)));
}

#[tokio::test]
async fn refuses_a_file_that_is_not_a_document_at_all() {
    assert_eq!(convert(b"this is not a docx".to_vec()).await, Err(DocxRefusal::Zip(ZipRefusal::NotAZip)));
    assert_eq!(DocxRefusal::Zip(ZipRefusal::NotAZip).as_str(), "not-a-zip");
}

#[tokio::test]
async fn refuses_an_oversized_document_by_size_alone() {
    let huge = vec![0x50; 11 * 1024 * 1024];
    assert_eq!(convert(huge).await, Err(DocxRefusal::TooLarge));
    assert_eq!(DocxRefusal::TooLarge.as_str(), "too-large");
}

// --- beyond the upstream suite ------------------------------------------------------------

#[tokio::test]
async fn a_zip_without_a_document_is_not_a_document() {
    let mut zip = build_docx(&para("x", None), &[]);
    // Rename word/document.xml in both headers: the main part is gone.
    let needle = b"word/document.xml";
    let mut i = 0;
    while let Some(pos) = zip[i..].windows(needle.len()).position(|w| w == needle) {
        zip[i + pos] = b'W';
        i += pos + 1;
    }
    assert_eq!(convert(zip).await, Err(DocxRefusal::NotADocument));
}

#[tokio::test]
async fn lists_tables_links_and_notes_come_out_like_mammoth() {
    let body = concat!(
        r#"<w:p><w:r><w:rPr><w:b/><w:i/></w:rPr><w:t xml:space="preserve">bold </w:t></w:r>"#,
        r#"<w:hyperlink w:anchor="top"><w:r><w:t>link</w:t></w:r></w:hyperlink></w:p>"#,
        r#"<w:tbl><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc></w:tr></w:tbl>"#,
        r#"<w:p><w:r><w:br/><w:t>x</w:t><w:tab/></w:r></w:p>"#,
    );
    let html = convert(build_docx(body, &[])).await.expect("converted").html;
    assert!(
        html.contains(
            r##"<main><p><strong><em>bold </em></strong><a href="#top">link</a></p><table><tr><td colspan="2"><p>a</p></td></tr></table><p><br />x	</p></main>"##
        ),
        "{html}"
    );
}

/// A 16x8 RGB PNG.
fn tiny_png() -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    }
    let (w, h) = (16u32, 8u32);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            raw.extend_from_slice(&[(x * 16) as u8, (y * 32) as u8, 128]);
        }
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), Compression::default());
    z.write_all(&raw).unwrap();
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z.finish().unwrap());
    chunk(&mut out, b"IEND", &[]);
    out
}

fn picture(rid: &str, descr: &str) -> String {
    format!(
        r#"<w:r><w:drawing><wp:inline xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"><wp:docPr id="1" name="p" descr="{descr}"/><a:graphic xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><a:graphicData><pic:pic xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:blipFill><a:blip xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" r:embed="{rid}"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>"#
    )
}

/// Pictures are re-encoded from the conversion thread through the async
/// libvips binding; that must work on a current-thread runtime too.
#[tokio::test]
async fn pictures_become_webp_data_uris_and_unreadable_ones_placeholders() {
    let rels = br#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rP" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/p.png"/><Relationship Id="rE" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/e.emf"/><Relationship Id="rM" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/missing.png"/></Relationships>"#;
    let body = format!("<w:p>{}{}{}</w:p>", picture("rP", "a picture"), picture("rE", "vector"), picture("rM", "gone"));
    let docx = build_docx(
        &body,
        &[
            ("word/_rels/document.xml.rels", rels.to_vec()),
            ("word/media/p.png", tiny_png()),
            ("word/media/e.emf", b"\x01\x00\x00\x00 not an image".to_vec()),
        ],
    );
    let result = convert(docx).await.expect("converted");
    assert_eq!((result.images, result.images_dropped), (1, 1));
    assert!(result.html.contains(r#"<p><img alt="a picture" src="data:image/webp;base64,UklGR"#), "{}", result.html);
    // The placeholder keeps `alt` first (it was set before the result's keys).
    assert!(result.html.contains(r#"<img alt="" src="" /></p>"#), "{}", result.html);
}

/// The Rust half of `tests/parity_media/docx_parity.py`: converts every file
/// in `DOCX_CORPUS` (a directory) and writes `{name: result}` JSON to
/// `DOCX_OUT`, the same shape the reference script prints.
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn parity_dump() {
    let dir = std::env::var("DOCX_CORPUS").expect("DOCX_CORPUS");
    let out = std::env::var("DOCX_OUT").expect("DOCX_OUT");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut results = serde_json::Map::new();
    for name in names {
        let bytes = std::fs::read(format!("{dir}/{name}")).unwrap();
        let value = match docx_to_html(Bytes::from(bytes)).await {
            Ok(p) => serde_json::json!({
                "ok": true, "html": p.html, "images": p.images, "imagesDropped": p.images_dropped,
            }),
            Err(r) => serde_json::json!({ "ok": false, "reason": r.as_str() }),
        };
        results.insert(name, value);
    }
    std::fs::write(out, serde_json::to_string(&results).unwrap()).unwrap();
}

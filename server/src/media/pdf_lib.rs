//! The part of pdf-lib 1.17.1 that `lib/media/convert.ts` uses, reproduced:
//! `PDFDocument.create()`, `embedJpg(jpeg)`, `addPage([w, h])`,
//! `page.drawImage(image, { x: 0, y: 0, width: w, height: h })`, `save()`.
//!
//! The output has the same structure as pdf-lib's, object for object: the
//! header with its four high bytes, objects 1-8 in the same order (pages,
//! catalog, info, the JPEG XObject, the page, the content stream, one object
//! stream holding the four dictionaries, a cross-reference stream), the same
//! dictionaries with their keys in pdf-lib's order, the same operators in the
//! content stream, and `save()`'s defaults (object streams on, every stream
//! but the image Flate-encoded).
//!
//! Two things can never match a given pdf-lib file byte for byte, and neither
//! could two runs of pdf-lib: the creation and modification dates in the info
//! dictionary (the moment of the call), and the XObject name, which pdf-lib
//! draws as `Image-` plus `Math.floor(Math.random() * 10 ** 10)`. The Flate
//! streams are compressed by miniz rather than by pako; they decompress to
//! the same bytes.

use std::io::Write;

use flate2::Compression;
use flate2::write::ZlibEncoder;

/// What pdf-lib's JpegEmbedder reads out of the SOF marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegInfo {
    pub bits_per_component: u8,
    pub width: u16,
    pub height: u16,
    /// `DeviceGray` / `DeviceRGB` / `DeviceCMYK` (1, 3 or 4 channels).
    pub color_space: &'static str,
}

/// SOF markers JpegEmbedder stops at.
const SOF_MARKERS: [u16; 15] = [
    0xffc0, 0xffc1, 0xffc2, 0xffc3, 0xffc5, 0xffc6, 0xffc7, 0xffc8, 0xffc9, 0xffca, 0xffcb, 0xffcc, 0xffcd, 0xffce,
    0xffcf,
];

fn u16_at(buf: &[u8], pos: usize) -> anyhow::Result<u16> {
    // DataView.getUint16 throws a RangeError past the end.
    let bytes = buf
        .get(pos..pos + 2)
        .ok_or_else(|| anyhow::anyhow!("RangeError: Offset is outside the bounds of the DataView"))?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn u8_at(buf: &[u8], pos: usize) -> anyhow::Result<u8> {
    buf.get(pos).copied().ok_or_else(|| anyhow::anyhow!("RangeError: Offset is outside the bounds of the DataView"))
}

/// `JpegEmbedder.for(imageData)`.
pub fn jpeg_info(data: &[u8]) -> anyhow::Result<JpegInfo> {
    if u16_at(data, 0)? != 0xffd8 {
        anyhow::bail!("SOI not found in JPEG");
    }
    let mut pos = 2;
    let mut marker = 0;
    while pos < data.len() {
        marker = u16_at(data, pos)?;
        pos += 2;
        if SOF_MARKERS.contains(&marker) {
            break;
        }
        pos += usize::from(u16_at(data, pos)?);
    }
    if !SOF_MARKERS.contains(&marker) {
        anyhow::bail!("Invalid JPEG");
    }
    pos += 2;
    let bits_per_component = u8_at(data, pos)?;
    pos += 1;
    let height = u16_at(data, pos)?;
    pos += 2;
    let width = u16_at(data, pos)?;
    pos += 2;
    let color_space = match u8_at(data, pos)? {
        1 => "DeviceGray",
        3 => "DeviceRGB",
        4 => "DeviceCMYK",
        _ => anyhow::bail!("Unknown JPEG channel."),
    };
    Ok(JpegInfo { bits_per_component, width, height, color_space })
}

/// `PDFHexString.fromText(text)`: UTF-16BE with a byte order mark, upper-case hex.
fn hex_text(text: &str) -> String {
    let mut out = String::from("<FEFF");
    for unit in text.encode_utf16() {
        out.push_str(&format!("{unit:04X}"));
    }
    out.push('>');
    out
}

/// `PDFString.fromDate(date)`: `D:YYYYMMDDHHmmSSZ` in UTC.
fn pdf_date(at: chrono::DateTime<chrono::Utc>) -> String {
    format!("({})", at.format("D:%Y%m%d%H%M%SZ"))
}

fn deflate(data: &[u8]) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).expect("writing to a Vec cannot fail");
    enc.finish().expect("writing to a Vec cannot fail")
}

/// The stream object syntax pdf-lib writes: `<<\n/K v\n...>>\nstream\n...\nendstream`.
fn stream_object(dict: &[(String, String)], contents: &[u8]) -> Vec<u8> {
    let mut out = dict_bytes(dict);
    out.extend_from_slice(b"\nstream\n");
    out.extend_from_slice(contents);
    out.extend_from_slice(b"\nendstream");
    out
}

fn dict_bytes(entries: &[(String, String)]) -> Vec<u8> {
    let mut s = String::from("<<\n");
    for (k, v) in entries {
        s.push_str(&format!("/{k} {v}\n"));
    }
    s.push_str(">>");
    s.into_bytes()
}

fn kv(k: &str, v: impl Into<String>) -> (String, String) {
    (k.to_string(), v.into())
}

/// `bytesFor(n).length`: bytes needed to hold `n` (at least one).
fn byte_width(mut n: u64) -> usize {
    let mut w = 1;
    while n > 0xff {
        n >>= 8;
        w += 1;
    }
    w
}

/// A one-page PDF holding `jpeg` at its own pixel size, as
/// `imageToPdf` builds it with pdf-lib. `image_name` is the XObject's
/// resource name (`Image-<random>` upstream), `now` the creation date.
pub fn single_jpeg_page_pdf(
    jpeg: &[u8],
    page_width: u32,
    page_height: u32,
    image_name: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> anyhow::Result<Vec<u8>> {
    let info = jpeg_info(jpeg)?;
    let producer = hex_text("pdf-lib (https://github.com/Hopding/pdf-lib)");
    let date = pdf_date(now);

    // Objects 1, 2, 3 and 5 go into the object stream (dictionaries,
    // generation 0); 4 (the image) and 6 (the content stream) are streams
    // and are written as they are.
    let pages = dict_bytes(&[kv("Type", "/Pages"), kv("Kids", "[ 5 0 R ]"), kv("Count", "1")]);
    let catalog = dict_bytes(&[kv("Type", "/Catalog"), kv("Pages", "1 0 R")]);
    let info_dict = dict_bytes(&[
        kv("Producer", producer.clone()),
        kv("ModDate", date.clone()),
        kv("Creator", producer),
        kv("CreationDate", date),
    ]);
    let resources = format!("<<\n/Font <<\n>>\n/XObject <<\n/{image_name} 4 0 R\n>>\n/ExtGState <<\n>>\n>>");
    let page = dict_bytes(&[
        kv("Type", "/Page"),
        kv("Parent", "1 0 R"),
        kv("Resources", resources),
        kv("MediaBox", format!("[ 0 0 {page_width} {page_height} ]")),
        kv("Annots", "[ ]"),
        kv("Contents", "[ 6 0 R ]"),
    ]);

    let mut image_dict = vec![
        kv("Type", "/XObject"),
        kv("Subtype", "/Image"),
        kv("BitsPerComponent", info.bits_per_component.to_string()),
        kv("Width", info.width.to_string()),
        kv("Height", info.height.to_string()),
        kv("ColorSpace", format!("/{}", info.color_space)),
        kv("Filter", "/DCTDecode"),
    ];
    // CMYK JPEG streams in PDF are typically stored complemented; pdf-lib
    // applies the swap as a hedge.
    if info.color_space == "DeviceCMYK" {
        image_dict.push(kv("Decode", "[ 1 0 1 0 1 0 1 0 ]"));
    }
    image_dict.push(kv("Length", jpeg.len().to_string()));
    let image = stream_object(&image_dict, jpeg);

    // drawImage: push, translate(x, y), rotateRadians(0), scale(w, h),
    // skewRadians(0, 0), drawObject, pop.
    let ops = format!(
        "q\n1 0 0 1 0 0 cm\n1 0 0 1 0 0 cm\n{page_width} 0 0 {page_height} 0 0 cm\n1 0 0 1 0 0 cm\n/{image_name} Do\nQ\n"
    );
    let ops = deflate(ops.as_bytes());
    let content = stream_object(&[kv("Filter", "/FlateDecode"), kv("Length", ops.len().to_string())], &ops);

    // The object stream: "num offset " pairs, then each object and a newline.
    let compressed: [(u32, &Vec<u8>); 4] = [(1, &pages), (2, &catalog), (3, &info_dict), (5, &page)];
    let mut offsets = String::new();
    let mut body = Vec::new();
    for (num, obj) in compressed {
        offsets.push_str(&format!("{num} {} ", body.len()));
        body.extend_from_slice(obj);
        body.push(b'\n');
    }
    let mut objstm_raw = offsets.clone().into_bytes();
    objstm_raw.extend_from_slice(&body);
    let objstm_data = deflate(&objstm_raw);
    let objstm = stream_object(
        &[
            kv("Filter", "/FlateDecode"),
            kv("Type", "/ObjStm"),
            kv("N", "4"),
            kv("First", offsets.len().to_string()),
            kv("Length", objstm_data.len().to_string()),
        ],
        &objstm_data,
    );

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"%PDF-1.7\n%\x81\x81\x81\x81\n\n");
    let write_obj = |out: &mut Vec<u8>, num: u32, obj: &[u8]| -> u64 {
        let at = out.len() as u64;
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(obj);
        out.extend_from_slice(b"\nendobj\n\n");
        at
    };
    let image_at = write_obj(&mut out, 4, &image);
    let content_at = write_obj(&mut out, 6, &content);
    let objstm_at = write_obj(&mut out, 7, &objstm);
    let xref_at = out.len() as u64;

    // The cross-reference stream, entries in the order pdf-lib adds them.
    // (type, field 2, field 3)
    let entries: [(u64, u64, u64); 9] = [
        (0, 0, 65535),
        (2, 7, 0),
        (2, 7, 1),
        (2, 7, 2),
        (1, image_at, 0),
        (2, 7, 3),
        (1, content_at, 0),
        (1, objstm_at, 0),
        (1, xref_at, 0),
    ];
    let widths = [
        entries.iter().map(|e| byte_width(e.0)).max().unwrap_or(1),
        entries.iter().map(|e| byte_width(e.1)).max().unwrap_or(1),
        entries.iter().map(|e| byte_width(e.2)).max().unwrap_or(1),
    ];
    let mut xref_raw = Vec::new();
    for (a, b, c) in entries {
        for (value, width) in [(a, widths[0]), (b, widths[1]), (c, widths[2])] {
            for i in (0..width).rev() {
                xref_raw.push((value >> (8 * i)) as u8);
            }
        }
    }
    let xref_data = deflate(&xref_raw);
    let xref = stream_object(
        &[
            kv("Size", "9"),
            kv("Root", "2 0 R"),
            kv("Info", "3 0 R"),
            kv("Filter", "/FlateDecode"),
            kv("Type", "/XRef"),
            kv("Length", xref_data.len().to_string()),
            kv("W", format!("[ {} {} {} ]", widths[0], widths[1], widths[2])),
            kv("Index", "[ 0 9 ]"),
        ],
        &xref_data,
    );
    write_obj(&mut out, 8, &xref);
    out.extend_from_slice(format!("startxref\n{xref_at}\n%%EOF").as_bytes());
    Ok(out)
}

/// `addRandomSuffix('Image', 10)`: `Image-` and a random integer below 10^10.
pub fn random_image_name() -> String {
    use rand::Rng;
    format!("Image-{}", rand::thread_rng().gen_range(0..10_000_000_000u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3x2 baseline JPEG (the one libvips writes for tests/../tiny.png).
    fn tiny_jpeg() -> Vec<u8> {
        let mut j = vec![0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00];
        j.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x02, 0x00, 0x03, 0x03]);
        j.extend_from_slice(&[0; 9]);
        j.extend_from_slice(&[0xff, 0xd9]);
        j
    }

    #[test]
    fn reads_the_sof_like_pdf_libs_embedder() {
        let info = jpeg_info(&tiny_jpeg()).unwrap();
        assert_eq!(info, JpegInfo { bits_per_component: 8, width: 3, height: 2, color_space: "DeviceRGB" });
        assert_eq!(jpeg_info(b"\x89PNG....").unwrap_err().to_string(), "SOI not found in JPEG");
    }

    #[test]
    fn writes_pdf_libs_structure() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-10-19T22:43:35Z").unwrap().with_timezone(&chrono::Utc);
        let pdf = single_jpeg_page_pdf(&tiny_jpeg(), 3, 2, "Image-7098480789", at).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.7\n%\x81\x81\x81\x81\n\n4 0 obj\n<<\n/Type /XObject\n/Subtype /Image\n"));
        assert!(pdf.ends_with(b"%%EOF"));
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("/Size 9\n/Root 2 0 R\n/Info 3 0 R\n/Filter /FlateDecode\n/Type /XRef\n"));
        assert!(text.contains("/W [ 1 2 2 ]\n/Index [ 0 9 ]"));
        assert!(text.contains("/Type /ObjStm\n/N 4\n/First 20\n"));
        assert_eq!(&hex_text("pd")[..13], "<FEFF00700064");
        assert_eq!(pdf_date(at), "(D:20261019224335Z)");
    }
}

//! mammoth `lib/docx/style-map.js` (the reading half): a style map embedded
//! in the document at `mammoth/style-map` is used before the default one.

use super::super::Failure;
use super::super::jszip::ZipFile;
use super::office_xml_reader::decode_utf8;

const STYLE_MAP_PATH: &str = "mammoth/style-map";

/// `readStyleMap(docxFile)`.
pub fn read_style_map(docx_file: &ZipFile) -> Result<Option<String>, Failure> {
    if !docx_file.exists(STYLE_MAP_PATH) {
        return Ok(None);
    }
    let bytes = docx_file.read(STYLE_MAP_PATH).map_err(|e| Failure::new(e.0))?;
    Ok(Some(decode_utf8(&bytes)))
}

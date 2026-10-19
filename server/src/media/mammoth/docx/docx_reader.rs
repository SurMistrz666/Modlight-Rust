//! mammoth `lib/docx/docx-reader.js`: which parts make up the document, read
//! in upstream's order (content types and part paths, styles, numbering,
//! notes and comments, then the body).

use std::sync::Arc;

use super::super::documents::{Document, NoteType};
use super::super::jszip::ZipFile;
use super::super::xml::XmlNode;
use super::super::zipfile::{join_path, split_path};
use super::super::{Ctx, Failure};
use super::body_reader::BodyReaderOptions;
use super::content_types_reader::{ContentTypes, default_content_types, read_content_types_from_xml};
use super::numbering_xml::{Numbering, default_numbering, read_numbering_xml};
use super::office_xml_reader::{read_xml_from_zip_file, require_element};
use super::relationships_reader::{Relationships, default_value, read_relationships};
use super::styles_reader::{Styles, default_styles, read_styles_xml};
use super::{comments_reader, document_xml_reader, notes_reader};

struct PartPaths {
    main_document: String,
    comments: String,
    endnotes: String,
    footnotes: String,
    numbering: String,
    styles: String,
}

/// `xmlFileReader({ filename, readElement, defaultValue })(zipFile)`.
fn xml_file_reader<T>(
    zip: &ZipFile,
    filename: &str,
    ctx: &Ctx,
    read_element: impl FnOnce(&XmlNode) -> Result<T, Failure>,
    default_value: impl FnOnce() -> T,
) -> Result<T, Failure> {
    match read_xml_from_zip_file(zip, filename, ctx.max_depth)? {
        Some(Some(node)) => read_element(&node),
        Some(None) | None => Ok(default_value()),
    }
}

fn read_relationships_file(zip: &ZipFile, filename: &str, ctx: &Ctx) -> Result<Relationships, Failure> {
    xml_file_reader(zip, filename, ctx, |n| read_relationships(require_element(n)?), default_value)
}

/// `relationshipsFilename(filename)`.
fn relationships_filename(filename: &str) -> String {
    let (dirname, basename) = split_path(filename);
    join_path(&[Some(dirname), Some("_rels"), Some(&format!("{basename}.rels"))])
}

fn strip_prefix(value: &str, prefix: &str) -> String {
    value.strip_prefix(prefix).unwrap_or(value).to_owned()
}

/// `findPartPath(options)`.
fn find_part_path(
    zip: &ZipFile,
    relationships: &Relationships,
    relationship_type: &str,
    base_path: &str,
    fallback_path: &str,
) -> String {
    relationships
        .find_targets_by_type(relationship_type)
        .iter()
        .map(|target| strip_prefix(&join_path(&[Some(base_path), target.as_deref()]), "/"))
        .find(|target| zip.exists(target))
        .unwrap_or_else(|| fallback_path.to_owned())
}

/// `findPartPaths(docxFile)`.
fn find_part_paths(zip: &ZipFile, ctx: &Ctx) -> Result<PartPaths, Failure> {
    let package_relationships = read_relationships_file(zip, "_rels/.rels", ctx)?;
    let main_document = find_part_path(
        zip,
        &package_relationships,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument",
        "",
        "word/document.xml",
    );
    if !zip.exists(&main_document) {
        return Err(Failure::new("Could not find main document part. Are you sure this is a valid .docx file?"));
    }
    let document_relationships = read_relationships_file(zip, &relationships_filename(&main_document), ctx)?;
    let base = split_path(&main_document).0.to_owned();
    let related = |name: &str| {
        find_part_path(
            zip,
            &document_relationships,
            &format!("http://schemas.openxmlformats.org/officeDocument/2006/relationships/{name}"),
            &base,
            &format!("word/{name}.xml"),
        )
    };
    Ok(PartPaths {
        comments: related("comments"),
        endnotes: related("endnotes"),
        footnotes: related("footnotes"),
        numbering: related("numbering"),
        styles: related("styles"),
        main_document,
    })
}

/// Shared state of `readXmlFileWithBody`.
struct Parts<'a> {
    zip: &'a Arc<ZipFile>,
    content_types: &'a ContentTypes,
    numbering: &'a Numbering,
    styles: &'a Styles,
    ctx: &'a Ctx,
}

impl Parts<'_> {
    /// `readXmlFileWithBody(filename, options, func)`: the part's
    /// relationships first (read even when the part is missing), then the
    /// part (`None` when missing or `undefined`).
    fn with_body<T>(
        &self,
        filename: &str,
        func: impl FnOnce(&BodyReaderOptions<'_>, Option<&XmlNode>) -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        let relationships = read_relationships_file(self.zip, &relationships_filename(filename), self.ctx)?;
        let options = BodyReaderOptions {
            relationships: &relationships,
            content_types: self.content_types,
            docx_file: self.zip,
            numbering: self.numbering,
            styles: self.styles,
            ctx: self.ctx,
        };
        let xml = read_xml_from_zip_file(self.zip, filename, self.ctx.max_depth)?;
        func(&options, xml.as_ref().and_then(|x| x.as_ref()))
    }
}

/// `read(docxFile)`.
pub fn read(zip: &Arc<ZipFile>, ctx: &Ctx) -> Result<Document, Failure> {
    let content_types = xml_file_reader(
        zip,
        "[Content_Types].xml",
        ctx,
        |n| read_content_types_from_xml(require_element(n)?),
        default_content_types,
    )?;
    let part_paths = find_part_paths(zip, ctx)?;
    let styles =
        xml_file_reader(zip, &part_paths.styles, ctx, |n| Ok(read_styles_xml(require_element(n)?)), default_styles)?;
    let numbering = xml_file_reader(
        zip,
        &part_paths.numbering,
        ctx,
        |n| read_numbering_xml(require_element(n)?, &styles),
        default_numbering,
    )?;
    let parts = Parts { zip, content_types: &content_types, numbering: &numbering, styles: &styles, ctx };

    let read_notes = |path: &str, note_type: NoteType| {
        parts.with_body(path, |options, xml| match xml {
            Some(node) => notes_reader::read_notes_xml(note_type, options, require_element(node)?),
            None => Ok(Vec::new()),
        })
    };
    let footnotes = read_notes(&part_paths.footnotes, NoteType::Footnote)?;
    let endnotes = read_notes(&part_paths.endnotes, NoteType::Endnote)?;
    let comments = parts.with_body(&part_paths.comments, |options, xml| match xml {
        Some(node) => comments_reader::read_comments_xml(options, require_element(node)?),
        None => Ok(Vec::new()),
    })?;
    let mut notes = footnotes;
    notes.extend(endnotes);

    parts.with_body(&part_paths.main_document, |options, xml| {
        let Some(node) = xml else {
            return Err(Failure::new("Cannot read properties of undefined (reading 'first')"));
        };
        document_xml_reader::convert_xml_to_document(options, require_element(node)?, notes, comments)
    })
}

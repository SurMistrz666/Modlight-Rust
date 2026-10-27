//! `lib/media/docx.ts`: turning a Word document into something a browser can
//! show.
//!
//! This is a PREVIEW, and the word is meant literally. mammoth reads the document
//! body and produces semantic HTML - headings, lists, tables, images. It does not
//! read the page layout, so what comes out is the document's CONTENT reflowed for
//! a screen, not a picture of the pages. Nothing in the UI may suggest otherwise,
//! and nothing here produces a PDF.
//!
//! WHY THE OUTPUT GOES IN THE SANDBOX RATHER THAN THROUGH A SANITISER
//!
//! mammoth escapes the text it extracts, so the obvious injection is already
//! handled. That is not the same as the output being trustworthy:
//!
//!  - hyperlinks come from the document's relationship targets, which are
//!    arbitrary strings the author controls, `javascript:` included;
//!  - `convertImage` is ours, but the default is a data: URI of whatever bytes
//!    the document carries, and "an image" in an OOXML part is only an image
//!    because the document says so;
//!  - the safety of the rest rests on mammoth's escaping being complete, in every
//!    version we will ever upgrade to, for every element it learns to emit.
//!
//! Writing a sanitiser instead means owning a security-critical allowlist
//! forever, and being wrong about it silently. This app already has a mechanism
//! for showing untrusted markup, it is the one used for uploaded HTML and SVG,
//! and it has been reasoned through once: a separate route, a ticket proving the
//! upload's own gates were cleared, a response CSP with `sandbox` that drops the
//! document into an opaque origin, and an iframe with no allow-same-origin. The
//! document cannot reach a cookie, our DOM, or a server action, whatever mammoth
//! emits and whatever the document contains.
//!
//! Reusing it costs one entry in a table. Writing a second mechanism costs a
//! second thing to be wrong. And this shape goes one step further than uploaded
//! HTML does: it renders with `sandbox=""` and no `allow-scripts`, like SVG,
//! because a converted document has no legitimate reason to run anything.
//!
//! The conversion itself is [`super::mammoth`] (mammoth 1.12.1 ported). It runs
//! on a thread of its own with a large stack (the readers recurse as deep as
//! the XML nests); each embedded picture is re-encoded through libvips from
//! there, one at a time, in document order, exactly as upstream's
//! `mapSeries` does, so the shared budget below drops the same pictures.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;

use crate::media::mammoth::{self, ConvertImageResult, js};
use crate::media::vips::{FailOn, Fit, Resize, Sharp, WebpOptions};
use crate::media::zip_guard::{ZipLimits, ZipRefusal, inspect_zip};

pub use crate::media::docx_support::{DocxKind, MAX_DOCX_BYTES, docx_support};

/// What the archive may expand to. A .docx is mostly XML, which compresses about
/// 10:1, so a legitimate 10 MB document lands well inside this; a bomb does not.
pub const ZIP_LIMITS: ZipLimits =
    ZipLimits { total_bytes: 80 * 1024 * 1024, entry_bytes: 40 * 1024 * 1024, entry_count: 4096 };

/// Total bytes of inlined imagery.
///
/// mammoth's default turns every embedded picture into a base64 data: URI with no
/// ceiling at all, so a document holding twelve 8 megapixel photographs becomes a
/// 40 MB HTML response - and base64 adds a third on top. Each image is re-encoded
/// below, and once this budget is spent the rest become placeholders rather than
/// being silently dropped.
pub const IMAGE_BUDGET_BYTES: usize = 3 * 1024 * 1024;

/// Longest edge an embedded image is scaled to. Wider than any reading column.
pub const IMAGE_MAX_EDGE: u32 = 1280;

/// Ceiling on the generated markup, in case a document is mostly text.
/// (Measured like JavaScript's `string.length`: UTF-16 code units.)
pub const MAX_HTML_BYTES: usize = 4 * 1024 * 1024;

/// Ceiling on a single embedded picture, in pixels.
///
/// Without it this is the one sharp call in the whole conversion path with no
/// pixel budget: it would fall back to the library default of 268 Mpx, four
/// times looser than the loosest ceiling the rest of this work derives from the
/// container's 2 GiB. A .docx is a ZIP, so an attacker controls both the size of
/// each embedded image and how many there are, and the enclosing byte budget
/// (IMAGE_BUDGET_BYTES) counts ENCODED output, which says nothing about how much
/// memory decoding the input takes. 24 Mpx is far beyond any picture that
/// belongs in a document and stays well inside one queue slot.
pub const IMAGE_MAX_PIXELS: u64 = 24_000_000;

/// The conversion thread's stack (see the module docs).
const CONVERSION_STACK_BYTES: usize = 256 * 1024 * 1024;

/// `DocxRefusal`: a `ZipRefusal`, or one of docx.ts's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocxRefusal {
    Zip(ZipRefusal),
    TooLarge,
    /// In the type upstream, never returned by `docxToHtml` (an OLE2 `.doc` is
    /// `not-a-zip` here; the caller tells the two apart with `docx_support`).
    LegacyDoc,
    NotADocument,
}

impl DocxRefusal {
    /// The upstream token (`result.reason`).
    pub fn as_str(self) -> &'static str {
        match self {
            DocxRefusal::Zip(z) => z.as_str(),
            DocxRefusal::TooLarge => "too-large",
            DocxRefusal::LegacyDoc => "legacy-doc",
            DocxRefusal::NotADocument => "not-a-document",
        }
    }
}

/// The `ok: true` side of `DocxResult`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocxPreview {
    pub html: String,
    pub images: u32,
    pub images_dropped: u32,
}

/// `DocxResult`: `{ ok: true, html, images, imagesDropped }` or
/// `{ ok: false, reason }`.
pub type DocxResult = Result<DocxPreview, DocxRefusal>;

/// Re-encode one embedded image to something bounded.
///
/// Three things at once, and each matters: the dimensions are capped so a photo
/// straight off a camera does not travel at full size; WebP at q72 is a third of
/// the original for this kind of content; and sharp strips EXIF, so the location
/// and camera serial in a picture somebody pasted into a report do not leave with
/// the preview. The same privacy rule the upload path already applies.
///
/// `(base64 data, mime)`, or `None` for anything libvips cannot read. A
/// document can carry EMF/WMF vector drawings, among other things; those become
/// placeholders.
pub async fn shrink_image(buf: Bytes) -> Option<(String, &'static str)> {
    let out = Sharp::new(buf)
        .fail_on(FailOn::None)
        .limit_input_pixels(Some(IMAGE_MAX_PIXELS))
        .rotate()
        .resize(Resize::to(Some(IMAGE_MAX_EDGE), Some(IMAGE_MAX_EDGE)).fit(Fit::Inside).without_enlargement(true))
        .webp(WebpOptions { quality: 72, ..Default::default() })
        .to_buffer()
        .await
        .ok()?;
    Some((STANDARD.encode(out), "image/webp"))
}

/// The attributes docx.ts gives a picture the budget refused.
fn placeholder() -> ConvertImageResult {
    Ok(vec![("src".into(), String::new()), ("alt".into(), String::new())])
}

/// Convert a .docx buffer to sandbox-ready HTML.
///
/// Never fails for a bad document: every failure is a reason the caller can turn
/// into a sentence, because "this file is a zip bomb" and "this file is not a
/// Word document" deserve different messages and neither deserves a 500.
pub async fn docx_to_html(buf: Bytes) -> DocxResult {
    if buf.len() > MAX_DOCX_BYTES {
        return Err(DocxRefusal::TooLarge);
    }
    let handle = tokio::runtime::Handle::current();
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("docx-preview".into())
        .stack_size(CONVERSION_STACK_BYTES)
        .spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| convert(buf, &handle)));
            let _ = tx.send(outcome);
        })
        .expect("spawn the docx conversion thread");
    match rx.await.expect("docx conversion thread ended without an answer") {
        Ok(result) => result,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// The body of `docxToHtml` after the size check, on the conversion thread.
fn convert(buf: Bytes, handle: &tokio::runtime::Handle) -> DocxResult {
    // Before mammoth, always. See zip_guard.rs: once JSZip has the buffer there is
    // no timeout that can save the process, because it expands synchronously.
    if let Err(reason) = inspect_zip(&buf, &ZIP_LIMITS) {
        return Err(DocxRefusal::Zip(reason));
    }

    let mut spent: usize = 0;
    let mut images: u32 = 0;
    let mut images_dropped: u32 = 0;

    let mut convert_image = |image: &mammoth::documents::Image| -> ConvertImageResult {
        if spent >= IMAGE_BUDGET_BYTES {
            images_dropped += 1;
            return placeholder();
        }
        let original = image.read()?;
        let Some((data, mime)) = handle.block_on(shrink_image(Bytes::from(original))) else {
            images_dropped += 1;
            return placeholder();
        };
        // base64 is four bytes for every three, and the budget is about what
        // travels over the wire, so it is charged on the encoded length.
        spent += data.len();
        if spent > IMAGE_BUDGET_BYTES {
            images_dropped += 1;
            return placeholder();
        }
        images += 1;
        Ok(vec![("src".into(), format!("data:{mime};base64,{data}"))])
    };

    // Every entry the guard inflated is within entry_bytes + 1 already; the
    // ceiling only stops an entry the guard never saw (see mammoth/jszip.rs).
    let body = match mammoth::convert_to_html(buf, ZIP_LIMITS.entry_bytes + 1, &mut convert_image) {
        Ok(body) => body,
        Err(_) => return Err(DocxRefusal::NotADocument),
    };

    if js::utf16_len(&body) > MAX_HTML_BYTES {
        return Err(DocxRefusal::Zip(ZipRefusal::TooBig));
    }

    Ok(DocxPreview { html: document_shell(&js::resolve_surrogates(&body)), images, images_dropped })
}

const SHELL_HEAD: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Document preview</title>
<style>
  :root { color-scheme: light dark; --ink: #1c1c1e; --paper: #ffffff; --muted: #6b7280; --rule: #e5e7eb; }
  @media (prefers-color-scheme: dark) {
    :root { --ink: #ededed; --paper: #121212; --muted: #a1a1a1; --rule: #262626; }
  }
  html { background: var(--paper); }
  body {
    margin: 0; padding: 2.5rem 1.5rem 4rem; background: var(--paper); color: var(--ink);
    font: 16px/1.7 ui-sans-serif, system-ui, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    -webkit-text-size-adjust: 100%;
  }
  main { max-width: 46rem; margin: 0 auto; }
  h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 2em 0 .6em; font-weight: 650; }
  h1 { font-size: 1.75rem; } h2 { font-size: 1.4rem; } h3 { font-size: 1.15rem; }
  p { margin: 0 0 1em; }
  a { color: inherit; text-underline-offset: 2px; }
  ul, ol { padding-left: 1.4em; margin: 0 0 1em; }
  li { margin: .25em 0; }
  img { max-width: 100%; height: auto; border-radius: 4px; display: block; margin: 1.25em 0; }
  img[src=""] {
    display: block; min-height: 4rem; border: 1px dashed var(--rule); border-radius: 4px;
    position: relative;
  }
  table { border-collapse: collapse; width: 100%; margin: 1.5em 0; font-size: .95em; }
  td, th { border: 1px solid var(--rule); padding: .45em .6em; text-align: left; vertical-align: top; }
  th { font-weight: 600; }
  blockquote { margin: 1.5em 0; padding-left: 1em; border-left: 3px solid var(--rule); color: var(--muted); }
  hr { border: 0; border-top: 1px solid var(--rule); margin: 2em 0; }
  pre, code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: .9em; }
  pre { overflow-x: auto; padding: .75em; border: 1px solid var(--rule); border-radius: 6px; }
</style>
</head>
<body><main>"#;

const SHELL_TAIL: &str = r#"</main></body>
</html>"#;

/// Wrap the fragment in a self-contained page.
///
/// Self-contained because it has to be: the frame is an opaque origin under
/// `default-src 'none'`, so it can load no stylesheet, no font and no script of
/// ours, and the app's theme tokens do not reach it. A document is paper, so it
/// looks like paper - but it follows the reader's colour scheme, because a white
/// rectangle appearing inside a dark panel reads as a bug.
///
/// Images with an empty `src` are the ones the budget refused; they are shown as
/// a labelled gap rather than as a broken-image icon.
pub fn document_shell(body: &str) -> String {
    let mut out = String::with_capacity(SHELL_HEAD.len() + body.len() + SHELL_TAIL.len());
    out.push_str(SHELL_HEAD);
    out.push_str(body);
    out.push_str(SHELL_TAIL);
    out
}

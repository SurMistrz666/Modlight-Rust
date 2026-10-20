//! mammoth `lib/zipfile.js` (and `lib/unzip.js`): the archive behind the
//! document (JSZip, ported in [`super::jszip`]) and the path helpers.

pub use super::jszip::ZipFile;

/// `splitPath(path)`: (dirname, basename).
pub fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        None => ("", path),
        Some(i) => (&path[..i], &path[i + 1..]),
    }
}

/// `joinPath(...paths)`: empty (and `undefined`) parts dropped, an absolute
/// part restarts the path; no normalisation.
pub fn join_path(paths: &[Option<&str>]) -> String {
    let mut relevant: Vec<&str> = Vec::new();
    for path in paths.iter().filter_map(|p| p.filter(|s| !s.is_empty())) {
        if path.starts_with('/') {
            relevant = vec![path];
        } else {
            relevant.push(path);
        }
    }
    relevant.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_like_mammoth() {
        assert_eq!(split_path("word/document.xml"), ("word", "document.xml"));
        assert_eq!(split_path("document.xml"), ("", "document.xml"));
        assert_eq!(join_path(&[Some(""), Some("word/document.xml")]), "word/document.xml");
        assert_eq!(join_path(&[Some("word"), Some("/x/y.xml")]), "/x/y.xml");
        assert_eq!(join_path(&[Some("word"), None]), "word");
        assert_eq!(
            join_path(&[Some("word"), Some("_rels"), Some("document.xml.rels")]),
            "word/_rels/document.xml.rels"
        );
    }
}

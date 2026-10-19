//! Port of `lib/paste-languages.ts`: the closed list of paste languages and
//! the extension each is stored under.

/// `LANG_EXT`, in upstream's key order.
pub const LANG_EXT: &[(&str, &str)] = &[
    ("text", "txt"),
    ("ts", "ts"),
    ("tsx", "tsx"),
    ("js", "js"),
    ("jsx", "jsx"),
    ("json", "json"),
    ("py", "py"),
    ("go", "go"),
    ("rs", "rs"),
    ("java", "java"),
    ("c", "c"),
    ("cpp", "cpp"),
    ("cs", "cs"),
    ("php", "php"),
    ("rb", "rb"),
    ("sh", "sh"),
    ("yml", "yml"),
    ("toml", "toml"),
    ("sql", "sql"),
    ("html", "html"),
    ("css", "css"),
    ("md", "md"),
    ("xml", "xml"),
    ("kt", "kt"),
    ("swift", "swift"),
    ("dockerfile", "dockerfile"),
    ("ini", "ini"),
    ("diff", "diff"),
];

/// `LANG_EXT[lang]` (exact key).
pub fn lang_ext(lang: &str) -> Option<&'static str> {
    LANG_EXT.iter().find(|(k, _)| *k == lang).map(|(_, e)| *e)
}

/// `PASTE_LANGUAGES`
pub fn paste_languages() -> Vec<&'static str> {
    LANG_EXT.iter().map(|(k, _)| *k).collect()
}

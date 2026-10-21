//! Text from an image (`OCRHelper`, `OCRViewModel`): the language list and the result's
//! handling. On macOS the recognizer is Apple's Vision framework instead of Windows OCR.
//! Vision names its languages with BCP 47 tags ("en-US", "zh-Hans"); the window lists
//! them by name, in the app's language. Upstream's default `Language`
//! ("en") and a tag synced from Windows ("pl") meet Vision's by their language subtag.

use image::RgbaImage;

/// `OCRLanguageOption`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanguageOption {
    pub name: String,
    pub tag: String,
}

/// Vision's tags and their names in English and Polish (macOS's own wording).
const NAMES: &[(&str, &str, &str)] = &[
    (
        "en-US",
        "English (United States)",
        "angielski (Stany Zjednoczone)",
    ),
    ("fr-FR", "French (France)", "francuski (Francja)"),
    ("it-IT", "Italian (Italy)", "włoski (Włochy)"),
    ("de-DE", "German (Germany)", "niemiecki (Niemcy)"),
    ("es-ES", "Spanish (Spain)", "hiszpański (Hiszpania)"),
    ("pt-BR", "Portuguese (Brazil)", "portugalski (Brazylia)"),
    ("zh-Hans", "Chinese, Simplified", "chiński uproszczony"),
    ("zh-Hant", "Chinese, Traditional", "chiński tradycyjny"),
    ("yue-Hans", "Cantonese, Simplified", "kantoński uproszczony"),
    ("yue-Hant", "Cantonese, Traditional", "kantoński tradycyjny"),
    (
        "ko-KR",
        "Korean (South Korea)",
        "koreański (Korea Południowa)",
    ),
    ("ja-JP", "Japanese (Japan)", "japoński (Japonia)"),
    ("ru-RU", "Russian (Russia)", "rosyjski (Rosja)"),
    ("uk-UA", "Ukrainian (Ukraine)", "ukraiński (Ukraina)"),
    ("th-TH", "Thai (Thailand)", "tajski (Tajlandia)"),
    ("vi-VT", "Vietnamese", "wietnamski"),
    (
        "ar-SA",
        "Arabic (Saudi Arabia)",
        "arabski (Arabia Saudyjska)",
    ),
    (
        "ars-SA",
        "Najdi Arabic (Saudi Arabia)",
        "arabski nadżdyjski (Arabia Saudyjska)",
    ),
    ("tr-TR", "Turkish (Türkiye)", "turecki (Turcja)"),
    (
        "id-ID",
        "Indonesian (Indonesia)",
        "indonezyjski (Indonezja)",
    ),
    ("cs-CZ", "Czech (Czechia)", "czeski (Czechy)"),
    ("da-DK", "Danish (Denmark)", "duński (Dania)"),
    ("nl-NL", "Dutch (Netherlands)", "niderlandzki (Holandia)"),
    ("no-NO", "Norwegian (Norway)", "norweski (Norwegia)"),
    (
        "nn-NO",
        "Norwegian Nynorsk (Norway)",
        "norweski (nynorsk) (Norwegia)",
    ),
    (
        "nb-NO",
        "Norwegian Bokmål (Norway)",
        "norweski (bokmål) (Norwegia)",
    ),
    ("ms-MY", "Malay (Malaysia)", "malajski (Malezja)"),
    ("pl-PL", "Polish (Poland)", "polski (Polska)"),
    ("ro-RO", "Romanian (Romania)", "rumuński (Rumunia)"),
    ("sv-SE", "Swedish (Sweden)", "szwedzki (Szwecja)"),
    ("fi-FI", "Finnish (Finland)", "fiński (Finlandia)"),
    ("hi-IN", "Hindi (India)", "hindi (Indie)"),
    ("mr-IN", "Marathi (India)", "marathi (Indie)"),
];

/// The languages Vision listed, named in the app's language and sorted by name
/// (`OrderBy(DisplayName, CurrentCultureIgnoreCase)`).
pub fn language_options(tags: &[String]) -> Vec<LanguageOption> {
    let polish = modlight_core::l10n::language() == modlight_core::l10n::Language::Polish;
    let mut options: Vec<LanguageOption> = tags
        .iter()
        .map(|tag| {
            let name = NAMES
                .iter()
                .find(|(t, _, _)| t.eq_ignore_ascii_case(tag))
                .map(|(_, en, pl)| if polish { *pl } else { *en })
                .unwrap_or(tag.as_str());
            LanguageOption {
                name: name.to_string(),
                tag: tag.clone(),
            }
        })
        .collect();
    options.sort_by_key(|o| o.name.to_lowercase());
    options
}

/// The language subtag of a BCP 47 tag ("zh-Hans" → "zh", "en" → "en").
fn primary(tag: &str) -> String {
    tag.split(['-', '_']).next().unwrap_or("").to_lowercase()
}

/// `SelectedLanguage`: the saved tag, else one with the same language, else the first.
pub fn select_language(options: &[LanguageOption], saved: &str) -> Option<usize> {
    if options.is_empty() {
        return None;
    }
    if let Some(i) = options
        .iter()
        .position(|o| o.tag.eq_ignore_ascii_case(saved))
    {
        return Some(i);
    }
    let wanted = primary(saved);
    options
        .iter()
        .position(|o| primary(&o.tag) == wanted)
        .or(Some(0))
}

/// `OCRHelper.OCR`'s scaling: `ImageHelpers.ScaleImageFast` by a factor of at least 1.
pub fn scale(image: &RgbaImage, factor: f64) -> RgbaImage {
    let factor = factor.max(1.0);
    if (factor - 1.0).abs() < f64::EPSILON {
        return image.clone();
    }
    let w = ((image.width() as f64 * factor).round() as u32).max(1);
    let h = ((image.height() as f64 * factor).round() as u32).max(1);
    image::imageops::resize(image, w, h, image::imageops::FilterType::Triangle)
}

/// `OCRInternal`'s ending: the lines joined with a space or line breaks.
pub fn join_lines(lines: &[String], single_line: bool) -> String {
    lines.join(if single_line { " " } else { "\n" })
}

/// `OCRServiceLinkOption.GenerateLink`: the URL pattern with the text, URL-encoded, in
/// place of `{0}`; nothing for empty text or an empty pattern.
pub fn service_link(pattern: &str, text: &str) -> Option<String> {
    if text.trim().is_empty() || pattern.trim().is_empty() {
        return None;
    }
    let encoded = modlight_core::tasks::naming::url_encode(text);
    Some(modlight_core::l10n::format(
        pattern,
        &[modlight_core::l10n::Arg::from(encoded)],
    ))
}

/// Where the recognised text of a picture file is written: the file with ".txt".
pub fn text_file_for(path: &std::path::Path) -> std::path::PathBuf {
    path.with_extension("txt")
}

#[cfg(test)]
mod tests {
    use super::*;
    use modlight_core::l10n::{Language, set_thread_language};

    fn tags() -> Vec<String> {
        ["en-US", "pl-PL", "zh-Hans", "de-DE", "xx-YY"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn names_and_order() {
        set_thread_language(Some(Language::English));
        let options = language_options(&tags());
        let names: Vec<&str> = options.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Chinese, Simplified",
                "English (United States)",
                "German (Germany)",
                "Polish (Poland)",
                "xx-YY"
            ]
        );
        set_thread_language(Some(Language::Polish));
        let options = language_options(&tags());
        assert_eq!(options[0].name, "angielski (Stany Zjednoczone)");
        set_thread_language(None);
    }

    #[test]
    fn saved_languages_meet_visions() {
        set_thread_language(Some(Language::English));
        let options = language_options(&tags());
        let tag = |saved: &str| select_language(&options, saved).map(|i| options[i].tag.as_str());
        assert_eq!(tag("en"), Some("en-US"));
        assert_eq!(tag("PL-pl"), Some("pl-PL"));
        assert_eq!(tag("zh-Hans-CN"), Some("zh-Hans"));
        assert_eq!(tag("fr"), Some("zh-Hans"));
        assert_eq!(select_language(&[], "en"), None);
        set_thread_language(None);
    }

    #[test]
    fn links_and_lines() {
        assert_eq!(
            service_link("https://www.google.com/search?q={0}", "zażółć gęślą").as_deref(),
            Some(
                "https://www.google.com/search?q=za%C5%BC%C3%B3%C5%82%C4%87%20g%C4%99%C5%9Bl%C4%85"
            )
        );
        assert_eq!(service_link("", "a"), None);
        assert_eq!(service_link("x{0}", "  "), None);
        let lines = vec!["one".to_string(), "two".to_string()];
        assert_eq!(join_lines(&lines, true), "one two");
        assert_eq!(join_lines(&lines, false), "one\ntwo");
        let image = RgbaImage::new(10, 4);
        assert_eq!(scale(&image, 2.5).dimensions(), (25, 10));
        assert_eq!(scale(&image, 0.5).dimensions(), (10, 4));
        assert_eq!(
            text_file_for(std::path::Path::new("/a/b.png")),
            std::path::PathBuf::from("/a/b.txt")
        );
    }
}

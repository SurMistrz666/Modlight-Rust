//! `ShareX/UploadInfoParser.cs`: the texts made after an upload (the clipboard format,
//! the "open URL" format, the notification text, the link formats of the after-upload
//! window) from a pattern with `$url`, `$filename` and the like. The name parser's `%`
//! codes are expanded first, with the `Default` type.

use super::file_helpers::path;
use super::name_parser::{NameParser, NameParserError, NameParserType};

/// `UploadInfoParser.HTMLLink`.
pub const HTML_LINK: &str = "<a href=\"$url\">$url</a>";
/// `UploadInfoParser.HTMLImage`.
pub const HTML_IMAGE: &str = "<img src=\"$url\">";
/// `UploadInfoParser.HTMLLinkedImage`.
pub const HTML_LINKED_IMAGE: &str = "<a href=\"$url\"><img src=\"$thumbnailurl\"></a>";
/// `UploadInfoParser.ForumLink`.
pub const FORUM_LINK: &str = "[url]$url[/url]";
/// `UploadInfoParser.ForumImage`.
pub const FORUM_IMAGE: &str = "[img]$url[/img]";
/// `UploadInfoParser.ForumLinkedImage`.
pub const FORUM_LINKED_IMAGE: &str = "[url=$url][img]$thumbnailurl[/img][/url]";
/// `UploadInfoParser.WikiImage`.
pub const WIKI_IMAGE: &str = "[$url]";
/// `UploadInfoParser.WikiLinkedImage`.
pub const WIKI_LINKED_IMAGE: &str = "[$url $thumbnailurl]";
/// `UploadInfoParser.MarkdownLink`.
pub const MARKDOWN_LINK: &str = "[$url]($url)";
/// `UploadInfoParser.MarkdownImage`.
pub const MARKDOWN_IMAGE: &str = "![]($url)";
/// `UploadInfoParser.MarkdownLinkedImage`.
pub const MARKDOWN_LINKED_IMAGE: &str = "[![]($thumbnailurl)]($url)";

/// The links of an upload (`UploadResult`'s URL fields); empty is upstream's null.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UploadLinks {
    pub url: String,
    pub thumbnail_url: String,
    pub deletion_url: String,
    pub shortened_url: String,
}

impl UploadLinks {
    /// `UploadResult.ToString()`: the short link when there is one, else the link.
    pub fn result_text(&self) -> &str {
        if !self.shortened_url.is_empty() {
            &self.shortened_url
        } else {
            &self.url
        }
    }
}

/// What `UploadInfoParser` reads of a task (`TaskInfo`); empty strings are upstream's
/// nulls, which it treats the same.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UploadInfo {
    /// `TaskInfo.Result`. With `None` the link codes (`$result`, `$url`, `$shorturl`,
    /// `$thumbnailurl`, `$deletionurl`) stay in the text as typed.
    pub result: Option<UploadLinks>,
    /// `TaskInfo.FileName`.
    pub file_name: String,
    /// `TaskInfo.FilePath`, also `$result` when there is no link.
    pub file_path: String,
    /// `TaskInfo.ThumbnailFilePath`.
    pub thumbnail_file_path: String,
    /// `TaskInfo.UploadDuration` in milliseconds; with `None` (nothing was uploaded)
    /// `$uploadtime` stays as typed.
    pub upload_duration_ms: Option<u64>,
}

/// `UploadInfoParser`: a [`NameParser`] of the `Default` type plus the `$` codes. Like
/// upstream, one parser kept for several texts carries its `%i` counter from one to the
/// next (`UploadInfoManager` keeps one; the task flow makes a new one per text).
#[derive(Clone, Debug, PartialEq)]
pub struct UploadInfoParser {
    pub name_parser: NameParser,
}

impl Default for UploadInfoParser {
    fn default() -> Self {
        UploadInfoParser::new()
    }
}

impl UploadInfoParser {
    pub fn new() -> UploadInfoParser {
        UploadInfoParser {
            name_parser: NameParser::new(NameParserType::Default),
        }
    }

    /// `Parse(TaskInfo info, string pattern)`. The codes are replaced one after another
    /// in upstream's order, which puts each code before the shorter one it starts with
    /// (`$filenamenoext` before `$filename`, `$thumbnailfilenamenoext` before
    /// `$thumbnailfilename`). A value holding a code replaced later gets that code
    /// replaced too, as upstream does.
    pub fn parse(&mut self, info: &UploadInfo, pattern: &str) -> Result<String, NameParserError> {
        if pattern.is_empty() {
            return Ok(String::new());
        }

        let mut pattern = self.name_parser.parse(pattern)?;

        if let Some(links) = &info.result {
            let mut result = links.result_text();
            if result.is_empty() && !info.file_path.is_empty() {
                result = &info.file_path;
            }
            pattern = pattern
                .replace("$result", result)
                .replace("$url", &links.url)
                .replace("$shorturl", &links.shortened_url)
                .replace("$thumbnailurl", &links.thumbnail_url)
                .replace("$deletionurl", &links.deletion_url);
        }

        let file_name_no_extension = if info.file_name.is_empty() {
            ""
        } else {
            path::get_file_name_without_extension(&info.file_name)
        };
        let folder_path = if info.file_path.is_empty() {
            None
        } else {
            path::get_directory_name(&info.file_path)
        };
        let folder_name = folder_path
            .as_deref()
            .map(path::get_file_name)
            .unwrap_or_default();
        let thumbnail = &info.thumbnail_file_path;
        let (thumbnail_name_no_extension, thumbnail_name) = if thumbnail.is_empty() {
            ("", "")
        } else {
            (
                path::get_file_name_without_extension(thumbnail),
                path::get_file_name(thumbnail),
            )
        };

        pattern = pattern
            .replace("$filenamenoext", file_name_no_extension)
            .replace("$filename", &info.file_name)
            .replace("$filepath", &info.file_path)
            .replace("$folderpath", folder_path.as_deref().unwrap_or_default())
            .replace("$foldername", folder_name)
            .replace("$thumbnailfilenamenoext", thumbnail_name_no_extension)
            .replace("$thumbnailfilename", thumbnail_name);

        if let Some(milliseconds) = info.upload_duration_ms {
            pattern = pattern.replace("$uploadtime", &milliseconds.to_string());
        }

        Ok(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture path as this system writes it: "/a/b" is "C:\a\b" on Windows.
    fn native(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{}", path.replace('/', "\\"))
        } else {
            path.to_string()
        }
    }

    fn uploaded() -> UploadInfo {
        UploadInfo {
            result: Some(UploadLinks {
                url: "https://m.example/f/abc.png".to_string(),
                thumbnail_url: "https://m.example/t/abc.jpg".to_string(),
                deletion_url: "https://m.example/d/abc".to_string(),
                shortened_url: String::new(),
            }),
            file_name: "Shot 2026.10.19.png".to_string(),
            file_path: native("/Users/me/Pictures/Modlight/2026-10/Shot 2026.10.19.png"),
            thumbnail_file_path: native("/Users/me/Pictures/Modlight/2026-10/Shot-thumb.jpg"),
            upload_duration_ms: Some(1234),
        }
    }

    fn parse(info: &UploadInfo, pattern: &str) -> String {
        UploadInfoParser::new().parse(info, pattern).unwrap()
    }

    #[test]
    fn every_variable() {
        let info = uploaded();
        assert_eq!(parse(&info, "$result"), "https://m.example/f/abc.png");
        assert_eq!(parse(&info, "$url"), "https://m.example/f/abc.png");
        assert_eq!(parse(&info, "[$shorturl]"), "[]");
        assert_eq!(parse(&info, "$thumbnailurl"), "https://m.example/t/abc.jpg");
        assert_eq!(parse(&info, "$deletionurl"), "https://m.example/d/abc");
        assert_eq!(parse(&info, "$filenamenoext"), "Shot 2026.10.19");
        assert_eq!(parse(&info, "$filename"), "Shot 2026.10.19.png");
        assert_eq!(
            parse(&info, "$filepath"),
            native("/Users/me/Pictures/Modlight/2026-10/Shot 2026.10.19.png")
        );
        assert_eq!(
            parse(&info, "$folderpath"),
            native("/Users/me/Pictures/Modlight/2026-10")
        );
        assert_eq!(parse(&info, "$foldername"), "2026-10");
        assert_eq!(parse(&info, "$thumbnailfilenamenoext"), "Shot-thumb");
        assert_eq!(parse(&info, "$thumbnailfilename"), "Shot-thumb.jpg");
        assert_eq!(parse(&info, "$uploadtime ms"), "1234 ms");
    }

    #[test]
    fn the_short_link_wins_and_the_file_stands_in_for_no_link() {
        let mut info = uploaded();
        info.result = Some(UploadLinks {
            shortened_url: "https://m.example/s/x".to_string(),
            ..info.result.clone().unwrap()
        });
        assert_eq!(
            parse(&info, "$result | $url | $shorturl"),
            "https://m.example/s/x | https://m.example/f/abc.png | https://m.example/s/x"
        );
        info.result = Some(UploadLinks::default());
        assert_eq!(parse(&info, "$result"), info.file_path);
        info.file_path.clear();
        assert_eq!(parse(&info, "<$result>"), "<>");
    }

    #[test]
    fn without_a_result_or_a_duration_the_codes_stay() {
        let mut info = uploaded();
        info.result = None;
        info.upload_duration_ms = None;
        assert_eq!(
            parse(
                &info,
                "$result $url $shorturl $thumbnailurl $deletionurl $uploadtime"
            ),
            "$result $url $shorturl $thumbnailurl $deletionurl $uploadtime"
        );
        assert_eq!(parse(&info, "$filename"), "Shot 2026.10.19.png");
    }

    #[test]
    fn empty_paths_give_empty_names() {
        let info = UploadInfo::default();
        assert_eq!(
            parse(
                &info,
                "[$filenamenoext|$filename|$filepath|$folderpath|$foldername|$thumbnailfilenamenoext|$thumbnailfilename]"
            ),
            "[||||||]"
        );
        let root = UploadInfo {
            file_path: native("/file.png"),
            ..UploadInfo::default()
        };
        assert_eq!(
            parse(&root, "[$folderpath|$foldername]"),
            format!("[{}|]", native("/"))
        );
        let bare = UploadInfo {
            file_path: "file.png".to_string(),
            ..UploadInfo::default()
        };
        assert_eq!(parse(&bare, "[$folderpath|$foldername]"), "[|]");
    }

    #[test]
    fn link_formats() {
        let info = uploaded();
        let url = "https://m.example/f/abc.png";
        let thumb = "https://m.example/t/abc.jpg";
        assert_eq!(
            parse(&info, HTML_LINK),
            format!("<a href=\"{url}\">{url}</a>")
        );
        assert_eq!(parse(&info, HTML_IMAGE), format!("<img src=\"{url}\">"));
        assert_eq!(
            parse(&info, HTML_LINKED_IMAGE),
            format!("<a href=\"{url}\"><img src=\"{thumb}\"></a>")
        );
        assert_eq!(parse(&info, FORUM_LINK), format!("[url]{url}[/url]"));
        assert_eq!(parse(&info, FORUM_IMAGE), format!("[img]{url}[/img]"));
        assert_eq!(
            parse(&info, FORUM_LINKED_IMAGE),
            format!("[url={url}][img]{thumb}[/img][/url]")
        );
        assert_eq!(parse(&info, WIKI_IMAGE), format!("[{url}]"));
        assert_eq!(parse(&info, WIKI_LINKED_IMAGE), format!("[{url} {thumb}]"));
        assert_eq!(parse(&info, MARKDOWN_LINK), format!("[{url}]({url})"));
        assert_eq!(parse(&info, MARKDOWN_IMAGE), format!("![]({url})"));
        assert_eq!(
            parse(&info, MARKDOWN_LINKED_IMAGE),
            format!("[![]({thumb})]({url})")
        );
    }

    #[test]
    fn name_codes_first_and_values_are_not_protected() {
        let mut info = uploaded();
        // The name parser runs first, on the pattern only.
        let mut parser = UploadInfoParser::new();
        parser.name_parser.now =
            Some(chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 10, 19, 8, 0, 0).unwrap());
        parser.name_parser.custom_time_zone = Some(super::super::CustomTimeZone::Utc);
        assert_eq!(
            parser.parse(&info, "%y: $filename").unwrap(),
            "2026: Shot 2026.10.19.png"
        );
        // %i counts on in a kept parser.
        assert_eq!(parser.parse(&info, "%i").unwrap(), "1");
        assert_eq!(parser.parse(&info, "%i").unwrap(), "2");
        // A link holding a later code gets it replaced, like upstream.
        info.result = Some(UploadLinks {
            url: "https://m.example/$filename".to_string(),
            ..UploadLinks::default()
        });
        assert_eq!(
            parse(&info, "$url"),
            "https://m.example/Shot 2026.10.19.png"
        );
        assert_eq!(parse(&info, ""), "");
    }
}

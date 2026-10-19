//! Port of `lib/media/link-formats.ts`: the one-click copy formats of a media.
//!
//! The browser builds these itself (components/viewer/MediaViewer.tsx and the
//! dashboard carry upstream's module verbatim); the server keeps the same
//! function so anything it renders agrees with what the panel copies.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkFormats {
    pub direct: String,
    pub raw: String,
    pub markdown: String,
    pub bbcode: String,
    pub html: String,
}

/// Build the one-click copy formats for a media. `is_image` decides whether
/// Markdown/BBCode/HTML use image syntax (pointing at the raw URL) or plain links.
pub fn build_link_formats(url: &str, raw_url: &str, filename: &str, is_image: bool) -> LinkFormats {
    LinkFormats {
        direct: url.to_string(),
        raw: raw_url.to_string(),
        markdown: if is_image { format!("![{filename}]({raw_url})") } else { format!("[{filename}]({url})") },
        bbcode: if is_image { format!("[img]{raw_url}[/img]") } else { format!("[url]{url}[/url]") },
        html: if is_image {
            format!("<img src=\"{raw_url}\" alt=\"{filename}\" />")
        } else {
            format!("<a href=\"{url}\">{filename}</a>")
        },
    }
}

/// `LINK_FORMAT_LABELS`, in the order the menus list them.
pub const LINK_FORMAT_LABELS: [(&str, &str); 5] =
    [("direct", "Direct link"), ("raw", "Raw link"), ("markdown", "Markdown"), ("bbcode", "BBCode"), ("html", "HTML")];

#[cfg(test)]
mod tests {
    //! tests/unit/link-formats.test.ts
    use super::*;

    const URL: &str = "https://x/u/c.png";
    const RAW: &str = "https://x/u/c/raw.png";

    #[test]
    fn image_formats_point_at_the_raw_url() {
        let f = build_link_formats(URL, RAW, "shot.png", true);
        assert_eq!(f.direct, URL);
        assert_eq!(f.raw, RAW);
        assert_eq!(f.markdown, "![shot.png](https://x/u/c/raw.png)");
        assert_eq!(f.bbcode, "[img]https://x/u/c/raw.png[/img]");
        assert!(f.html.contains("<img"));
        assert_eq!(f.html, "<img src=\"https://x/u/c/raw.png\" alt=\"shot.png\" />");
    }

    #[test]
    fn non_image_formats_use_plain_links() {
        let f = build_link_formats(URL, RAW, "shot.png", false);
        assert_eq!(f.markdown, "[shot.png](https://x/u/c.png)");
        assert_eq!(f.bbcode, "[url]https://x/u/c.png[/url]");
        assert!(f.html.contains("<a href"));
        assert_eq!(f.html, "<a href=\"https://x/u/c.png\">shot.png</a>");
    }

    #[test]
    fn labels_cover_every_format() {
        let f = serde_json::to_value(build_link_formats(URL, RAW, "a", true)).unwrap();
        let keys: Vec<&str> = f.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, LINK_FORMAT_LABELS.iter().map(|(k, _)| *k).collect::<Vec<_>>());
    }
}

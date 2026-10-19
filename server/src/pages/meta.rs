//! Page metadata: Next's `Metadata` (the subset upstream uses), merged with the
//! root layout's defaults and rendered into `<head>` tags in Next's order and
//! format, so link-preview bots (which read only the first HTML response) see
//! what they saw upstream.
//!
//! Root layout (app/layout.tsx): `title: { default: APP_NAME, template:
//! "%s - APP_NAME" }`, the product description, `applicationName`, and the
//! icons from `app/icon.svg` + `app/apple-icon.png`.
//!
//! Inheritance follows Next: a field the page leaves out is inherited from the
//! root, `null` (here [`Field::Clear`]) removes it, a plain-string title goes
//! through the template and `{ absolute }` does not. When `openGraph` is set
//! and `twitter` lacks a title, description or images, Next fills them in from
//! Open Graph; so does [`resolve`].
//!
//! Anything the structured fields cannot express goes into [`PageMeta::extra`]
//! as raw tags, appended after the generated ones.

use serde::Serialize;

use crate::config::Config;

/// The root layout's description (app/layout.tsx).
pub const ROOT_DESCRIPTION: &str = "Fast, modern, self-hosted file & screenshot sharing with rock-solid large-file uploads and first-class ShareX support.";

/// A metadata field: inherited, explicitly removed (`null`), or set.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub enum Field<T> {
    #[default]
    Inherit,
    Clear,
    Set(T),
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub enum Title {
    /// Use the parent's (root: APP_NAME).
    #[default]
    Inherit,
    /// `title: "x"`: rendered through the root template (`x - APP_NAME`).
    Templated(String),
    /// `title: { absolute: "x" }`
    Absolute(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Robots {
    pub index: bool,
    pub follow: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Alternates {
    pub canonical: Option<String>,
    /// `types: { "application/json+oembed": href, ... }` in order.
    pub types: Vec<(String, String)>,
}

/// One `og:image` / `og:video`: `url` first, then `(property suffix, value)`
/// pairs in the order the page listed them (`secure_url`, `width`, ...).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct OgMedia {
    pub url: String,
    pub props: Vec<(String, String)>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct OpenGraph {
    /// `website`, `profile`, `video.other`, ...
    pub og_type: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub url: Option<String>,
    pub site_name: Option<String>,
    pub images: Vec<OgMedia>,
    pub videos: Vec<OgMedia>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Twitter {
    pub card: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub images: Option<Vec<String>>,
}

/// A raw head tag for what the structured fields do not cover.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum HeadTag {
    /// `<meta name=.. content=..>`
    MetaName(String, String),
    /// `<meta property=.. content=..>`
    MetaProperty(String, String),
    /// `<link rel=.. href=.. [type=..]>`
    Link { rel: String, href: String, link_type: Option<String> },
}

/// What a page's `metadata` / `generateMetadata` returned.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PageMeta {
    pub title: Title,
    pub description: Field<String>,
    pub application_name: Field<String>,
    pub robots: Option<Robots>,
    pub alternates: Option<Alternates>,
    pub open_graph: Option<OpenGraph>,
    pub twitter: Option<Twitter>,
    pub extra: Vec<HeadTag>,
}

impl PageMeta {
    /// `{ title: "x" }`
    pub fn titled(title: impl Into<String>) -> PageMeta {
        PageMeta { title: Title::Templated(title.into()), ..Default::default() }
    }

    /// `robots: { index: false, follow: false }`
    pub fn noindex(mut self) -> PageMeta {
        self.robots = Some(Robots { index: false, follow: false });
        self
    }
}

/// The final head of one response (also sent to the SPA as `meta`).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedHead {
    pub title: String,
    pub description: Option<String>,
    pub application_name: Option<String>,
    /// `noindex, nofollow` etc.
    pub robots: Option<String>,
    #[serde(skip)]
    pub alternates: Option<Alternates>,
    #[serde(skip)]
    pub open_graph: Option<OpenGraph>,
    #[serde(skip)]
    pub twitter: Option<Twitter>,
    #[serde(skip)]
    pub extra: Vec<HeadTag>,
    #[serde(skip)]
    pub icons: Vec<HeadTag>,
}

fn icon_tags() -> Vec<HeadTag> {
    vec![
        HeadTag::Link { rel: "icon".into(), href: "/icon.svg".into(), link_type: Some("image/svg+xml".into()) },
        HeadTag::Link {
            rel: "apple-touch-icon".into(),
            href: "/apple-icon.png".into(),
            link_type: Some("image/png".into()),
        },
    ]
}

fn robots_string(r: Robots) -> String {
    format!("{}, {}", if r.index { "index" } else { "noindex" }, if r.follow { "follow" } else { "nofollow" })
}

/// Merge page metadata onto the root layout's. `not_found` renders what Next
/// renders for a not-found boundary: the root metadata plus `robots: noindex`.
pub fn resolve(config: &Config, page: &PageMeta, not_found: bool) -> ResolvedHead {
    let app = &config.app_name;
    if not_found {
        return ResolvedHead {
            title: app.clone(),
            description: Some(ROOT_DESCRIPTION.to_string()),
            application_name: Some(app.clone()),
            robots: Some("noindex".to_string()),
            icons: icon_tags(),
            ..Default::default()
        };
    }
    let title = match &page.title {
        Title::Inherit => app.clone(),
        Title::Templated(t) => format!("{t} - {app}"),
        Title::Absolute(t) => t.clone(),
    };
    let field = |f: &Field<String>, root: &str| match f {
        Field::Inherit => Some(root.to_string()),
        Field::Clear => None,
        Field::Set(v) => Some(v.clone()),
    };
    let mut twitter = page.twitter.clone();
    if let Some(og) = &page.open_graph {
        let mut tw = twitter.take().unwrap_or_default();
        let had_any = page.twitter.is_some();
        if tw.title.is_none() {
            tw.title = og.title.clone();
        }
        if tw.description.is_none() {
            tw.description = og.description.clone();
        }
        if tw.images.is_none() && !og.images.is_empty() {
            tw.images = Some(og.images.iter().map(|i| i.url.clone()).collect());
        }
        if tw.card.is_none() {
            let has_images = tw.images.as_ref().is_some_and(|i| !i.is_empty());
            tw.card = Some(if has_images { "summary_large_image" } else { "summary" }.to_string());
        }
        if had_any || tw.title.is_some() || tw.description.is_some() || tw.images.is_some() {
            twitter = Some(tw);
        }
    }
    ResolvedHead {
        title,
        description: field(&page.description, ROOT_DESCRIPTION),
        application_name: field(&page.application_name, app),
        robots: page.robots.map(robots_string),
        alternates: page.alternates.clone(),
        open_graph: page.open_graph.clone(),
        twitter,
        extra: page.extra.clone(),
        icons: icon_tags(),
    }
}

/// Attribute-value escaping as React renders it.
pub fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

fn meta_name(name: &str, content: &str) -> String {
    format!("<meta name=\"{}\" content=\"{}\"/>", escape_attr(name), escape_attr(content))
}

fn meta_property(property: &str, content: &str) -> String {
    format!("<meta property=\"{}\" content=\"{}\"/>", escape_attr(property), escape_attr(content))
}

fn link(rel: &str, href: &str, link_type: Option<&str>, extra: &str) -> String {
    match link_type {
        Some(t) => format!(
            "<link rel=\"{}\" href=\"{}\"{extra} type=\"{}\"/>",
            escape_attr(rel),
            escape_attr(href),
            escape_attr(t)
        ),
        None => format!("<link rel=\"{}\" href=\"{}\"{extra}/>", escape_attr(rel), escape_attr(href)),
    }
}

fn render_tag(tag: &HeadTag) -> String {
    match tag {
        HeadTag::MetaName(n, c) => meta_name(n, c),
        HeadTag::MetaProperty(p, c) => meta_property(p, c),
        HeadTag::Link { rel, href, link_type } => link(rel, href, link_type.as_deref(), ""),
    }
}

/// The tags, in Next's order: robots (not-found), title, basic meta,
/// alternates, Open Graph, Twitter, extras, icons.
pub fn render_head(head: &ResolvedHead) -> String {
    let mut out: Vec<String> = Vec::new();
    out.push(format!("<title>{}</title>", escape_text(&head.title)));
    if let Some(d) = &head.description {
        out.push(meta_name("description", d));
    }
    if let Some(a) = &head.application_name {
        out.push(meta_name("application-name", a));
    }
    if let Some(r) = &head.robots {
        out.push(meta_name("robots", r));
    }
    if let Some(alt) = &head.alternates {
        if let Some(c) = &alt.canonical {
            out.push(link("canonical", c, None, ""));
        }
        for (t, href) in &alt.types {
            out.push(format!("<link rel=\"alternate\" type=\"{}\" href=\"{}\"/>", escape_attr(t), escape_attr(href)));
        }
    }
    if let Some(og) = &head.open_graph {
        if let Some(t) = &og.title {
            out.push(meta_property("og:title", t));
        }
        if let Some(d) = &og.description {
            out.push(meta_property("og:description", d));
        }
        if let Some(u) = &og.url {
            out.push(meta_property("og:url", u));
        }
        if let Some(s) = &og.site_name {
            out.push(meta_property("og:site_name", s));
        }
        for img in &og.images {
            out.push(meta_property("og:image", &img.url));
            for (k, v) in &img.props {
                out.push(meta_property(&format!("og:image:{k}"), v));
            }
        }
        for vid in &og.videos {
            out.push(meta_property("og:video", &vid.url));
            for (k, v) in &vid.props {
                out.push(meta_property(&format!("og:video:{k}"), v));
            }
        }
        if let Some(t) = &og.og_type {
            out.push(meta_property("og:type", t));
        }
    }
    if let Some(tw) = &head.twitter {
        if let Some(c) = &tw.card {
            out.push(meta_name("twitter:card", c));
        }
        if let Some(t) = &tw.title {
            out.push(meta_name("twitter:title", t));
        }
        if let Some(d) = &tw.description {
            out.push(meta_name("twitter:description", d));
        }
        for img in tw.images.iter().flatten() {
            out.push(meta_name("twitter:image", img));
        }
    }
    for tag in &head.extra {
        out.push(render_tag(tag));
    }
    out.push("<link rel=\"icon\" href=\"/icon.svg\" sizes=\"any\" type=\"image/svg+xml\"/>".to_string());
    out.push(
        "<link rel=\"apple-touch-icon\" href=\"/apple-icon.png\" sizes=\"180x180\" type=\"image/png\"/>".to_string(),
    );
    out.join("")
}

/// Text-node escaping as React renders it.
pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config() -> Config {
        Config::from_env_map(&HashMap::new()).unwrap()
    }

    /// The login page of the reference server, minus the framework tags.
    #[test]
    fn login_head_matches_the_reference() {
        let head = resolve(&config(), &PageMeta::titled("Sign in"), false);
        assert_eq!(
            render_head(&head),
            "<title>Sign in - Modlight</title><meta name=\"description\" content=\"Fast, modern, self-hosted file &amp; screenshot sharing with rock-solid large-file uploads and first-class ShareX support.\"/><meta name=\"application-name\" content=\"Modlight\"/><link rel=\"icon\" href=\"/icon.svg\" sizes=\"any\" type=\"image/svg+xml\"/><link rel=\"apple-touch-icon\" href=\"/apple-icon.png\" sizes=\"180x180\" type=\"image/png\"/>"
        );
    }

    #[test]
    fn not_found_is_root_metadata_plus_noindex() {
        let head = resolve(&config(), &PageMeta::titled("ignored"), true);
        assert_eq!(head.title, "Modlight");
        assert_eq!(head.robots.as_deref(), Some("noindex"));
    }

    #[test]
    fn absolute_titles_and_cleared_fields() {
        let page = PageMeta {
            title: Title::Absolute("shot.png · example".into()),
            description: Field::Clear,
            application_name: Field::Clear,
            open_graph: Some(OpenGraph {
                og_type: Some("website".into()),
                title: Some("shot.png".into()),
                description: Some("1920 × 1080".into()),
                images: vec![OgMedia { url: "https://x/raw".into(), props: vec![("width".into(), "1920".into())] }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let head = resolve(&config(), &page, false);
        assert_eq!(head.title, "shot.png · example");
        assert!(head.description.is_none() && head.application_name.is_none());
        let tw = head.twitter.clone().unwrap();
        assert_eq!(tw.card.as_deref(), Some("summary_large_image"));
        assert_eq!(tw.images.unwrap(), vec!["https://x/raw".to_string()]);
        let html = render_head(&head);
        assert!(html.contains("<meta property=\"og:image:width\" content=\"1920\"/>"));
        assert!(html.contains("<meta name=\"twitter:title\" content=\"shot.png\"/>"));
    }
}

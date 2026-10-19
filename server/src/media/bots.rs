//! Port of `lib/media/bots.ts`: who is asking, a reader or a link preview?
//!
//! These patterns used to live inside the preview page, which is why the page
//! knew not to spend a view on a crawler and the raw route did not. That gap was
//! not theoretical: the og:image of a picture points at /raw, so a single unfurl
//! by X or WhatsApp fetched the bytes and consumed a view - and a burn-after-view
//! link died in the chat window before its recipient ever clicked it.
//!
//! The upstream regular expressions are case-insensitive alternations of plain
//! words, tested anywhere in the string; without the `u` flag JavaScript folds
//! case for ASCII only, so an ASCII-lowercased substring search is the same test.

/// Chat clients that turn a *bare file URL* into a native media embed - the image
/// or the video itself, with no card, no title and no site name around it.
///
/// Handing them the HTML preview page instead is what produces a link card:
/// Open Graph is the best a document can do. Pointing them straight at the bytes
/// is what makes Discord inline the picture and give an mp4 a real player.
/// Everything not listed here (X, Facebook, LinkedIn, Bluesky...) only
/// understands markup, so those keep getting the page and its OG tags.
pub const DIRECT_MEDIA_BOTS: &[&str] = &[
    "discordbot",
    "telegrambot",
    "slackbot",
    "slack-imgproxy",
    "skypeuripreview",
    "mattermost",
    "synapse",
    "revoltbot",
    "guildedbot",
];

/// Every unfurler and crawler we recognise beyond [`DIRECT_MEDIA_BOTS`]. Used to
/// keep automated previews out of the statistics - and, more importantly, to
/// stop a link preview from spending a budget that was meant for a human.
pub const OTHER_PREVIEW_BOTS: &[&str] = &[
    "whatsapp",
    "twitterbot",
    "facebookexternalhit",
    "facebot",
    "linkedinbot",
    "redditbot",
    "embedly",
    "iframely",
    "vkshare",
    "tumblr",
    "flipboard",
    "bitlybot",
    "nuzzel",
    "bufferbot",
    "quora link preview",
    "showyoubot",
    "outbrain",
    "googlebot",
    "bingbot",
    "applebot",
    "duckduckbot",
    "yandexbot",
    "petalbot",
    "semrushbot",
    "ahrefsbot",
    "cardyb",
    "w3c_validator",
];

fn matches_any(user_agent: &str, words: &[&str]) -> bool {
    let ua = user_agent.to_ascii_lowercase();
    words.iter().any(|w| ua.contains(w))
}

/// `isPreviewBot(userAgent)`: an empty or missing agent is a reader.
pub fn is_preview_bot(user_agent: Option<&str>) -> bool {
    match user_agent {
        Some(ua) if !ua.is_empty() => matches_any(ua, DIRECT_MEDIA_BOTS) || matches_any(ua, OTHER_PREVIEW_BOTS),
        _ => false,
    }
}

/// `isDirectMediaBot(userAgent)`
pub fn is_direct_media_bot(user_agent: Option<&str>) -> bool {
    match user_agent {
        Some(ua) if !ua.is_empty() => matches_any(ua, DIRECT_MEDIA_BOTS),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    //! tests/unit/preview-bots.test.ts (the source-scanning half is in
    //! `routes::media_action`'s tests, which pin the same rules on the port).
    use super::*;

    const UNFURLERS: &[&str] = &[
        "Mozilla/5.0 (compatible; Discordbot/2.0; +https://discordapp.com)",
        "TelegramBot (like TwitterBot)",
        "Slackbot-LinkExpanding 1.0 (+https://api.slack.com/robots)",
        "Mozilla/5.0 (compatible; Twitterbot/1.0)",
        "facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)",
        "WhatsApp/2.23.20.0 A",
        "LinkedInBot/1.0 (compatible; Mozilla/5.0; Jakarta Commons-HttpClient/3.1)",
        "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
        "Mozilla/5.0 (compatible; Discordbot/2.0)",
    ];

    const HUMANS: &[&str] = &[
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36",
        "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
        "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Mobile/15E148",
        "curl/8.5.0",
        "ShareX/16.1.0",
    ];

    #[test]
    fn recognises_unfurlers() {
        for ua in UNFURLERS {
            assert!(is_preview_bot(Some(ua)), "{ua}");
        }
    }

    #[test]
    fn leaves_humans_alone() {
        for ua in HUMANS {
            assert!(!is_preview_bot(Some(ua)), "{ua}");
        }
    }

    #[test]
    fn treats_a_missing_user_agent_as_a_reader() {
        // Cheap to under-detect (one extra view); expensive to over-detect (a real
        // visitor's view goes unrecorded).
        assert!(!is_preview_bot(None));
        assert!(!is_preview_bot(Some("")));
    }

    #[test]
    fn keeps_the_media_embedding_subset_inside_the_wider_set() {
        assert!(is_direct_media_bot(Some("Discordbot/2.0")));
        // X reads markup only, so it gets the page, never a redirect to the bytes.
        assert!(!is_direct_media_bot(Some("Twitterbot/1.0")));
        assert!(is_preview_bot(Some("Twitterbot/1.0")));
        for ua in DIRECT_MEDIA_BOTS {
            assert!(is_preview_bot(Some(ua)));
        }
    }

    #[test]
    fn case_folding_is_ascii_only_like_a_non_unicode_regexp() {
        assert!(is_preview_bot(Some("QUORA LINK PREVIEW/1")));
        assert!(is_preview_bot(Some("xx W3C_Validator yy")));
        // U+017F LONG S upper-cases to "S", but a non-unicode RegExp never folds
        // a non-ASCII character onto an ASCII one.
        assert!(!is_direct_media_bot(Some("di\u{17F}cordbot")));
    }
}

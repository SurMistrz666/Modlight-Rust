//! The account-related parts of the upstream unit tests that the modules'
//! own tests did not cover yet (kept here, against the public API, so the
//! modules other ports own stay untouched):
//! `tests/unit/naming.test.ts`, `naming-urls.test.ts`, `naming-schemes.test.ts`,
//! `naming-domains.test.ts`, `media-urls-owner.test.ts`, `background.test.ts`,
//! `avatar.test.ts`, `quota.test.ts`, and `server-action-exports.test.ts`
//! (as: every export of the upstream `account` module is a registered action).
//!
//! Not ported here: the generators (`slugifyName`, `buildCodeBase`,
//! `allocateCode`, `wordCode`, ...), `mediaPath` / short-code scoping,
//! `planAvatar` and `isAcceptedAvatarType` belong to the upload and library
//! ports, which own those functions.

use std::collections::HashMap;

use modlight_server::config::Config;

fn config(vars: &[(&str, &str)]) -> Config {
    let map: HashMap<String, String> = vars.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
    Config::from_env_map(&map).expect("config")
}

const HOST: &str = "https://host.example.com";
const H: &str = "https://h.example.com";
const S: &str = "https://s.example.com";

mod naming {
    //! tests/unit/naming.test.ts (checkTag), naming-urls.test.ts
    //! (isReservedTag), naming-schemes.test.ts (clamps, scheme list).
    use modlight_server::naming::{
        NAMING_SCHEMES, TagProblem, check_tag, clamp_random_length, clamp_word_count, is_naming_scheme, is_reserved_tag,
    };

    #[test]
    fn accepts_the_legacy_production_tags() {
        // These already exist and must keep working after migration.
        for tag in ["alicja", "bobby", "root", "hIto3", "kiNo4", "PaLU8", "wIke5"] {
            assert_eq!(check_tag(tag), Ok(()), "{tag}");
        }
    }

    #[test]
    fn rejects_tags_that_would_shadow_an_app_route() {
        assert_eq!(check_tag("dashboard"), Err(TagProblem::Reserved));
        assert_eq!(check_tag("api"), Err(TagProblem::Reserved));
        assert_eq!(check_tag("Login"), Err(TagProblem::Reserved)); // case-insensitive
        assert_eq!(check_tag("s"), Err(TagProblem::TooShort));
    }

    #[test]
    fn rejects_unsafe_path_characters() {
        for tag in ["a/b", "..", "a b", "-lead"] {
            assert_eq!(check_tag(tag), Err(TagProblem::Charset), "{tag}");
        }
    }

    #[test]
    fn enforces_length_bounds() {
        assert_eq!(check_tag("x"), Err(TagProblem::TooShort));
        assert_eq!(check_tag(&"x".repeat(33)), Err(TagProblem::TooLong));
        assert_eq!(check_tag(&"x".repeat(32)), Ok(()));
        // UTF-16 code units, like `.length`: one astral character is two.
        assert_eq!(check_tag("\u{1F600}"), Err(TagProblem::Charset));
        assert_eq!(TagProblem::TooLong.as_str(), "too-long");
    }

    #[test]
    fn reserved_tags_guard_the_routes_that_exist_today() {
        for name in ["login", "dashboard", "api", "s", "u", "a", "welcome", "trash"] {
            assert!(is_reserved_tag(name), "{name}");
        }
    }

    #[test]
    fn reserved_tags_are_case_insensitive() {
        assert!(is_reserved_tag("Login"));
        assert!(is_reserved_tag("DASHBOARD"));
    }

    #[test]
    fn reserved_tags_match_whole_segments_only() {
        // A tagless upload called login-screen.png must not be blocked.
        assert!(!is_reserved_tag("login-screen"));
        assert!(!is_reserved_tag("apis"));
        assert!(!is_reserved_tag("alicja"));
    }

    #[test]
    fn keeps_random_codes_at_five_characters_or_more() {
        assert_eq!(clamp_random_length(Some(1.0)), 5);
        assert_eq!(clamp_random_length(Some(0.0)), 5);
        assert_eq!(clamp_random_length(Some(-3.0)), 5);
        assert_eq!(clamp_random_length(None), 6);
        assert_eq!(clamp_random_length(Some(999.0)), 32);
        assert_eq!(clamp_random_length(Some(7.0)), 7);
    }

    #[test]
    fn keeps_word_counts_within_two_to_four() {
        assert_eq!(clamp_word_count(Some(0.0)), 2);
        assert_eq!(clamp_word_count(None), 2);
        assert_eq!(clamp_word_count(Some(9.0)), 4);
        assert_eq!(clamp_word_count(Some(3.0)), 3);
        // Math.round: halves go up, so 2.5 is 3.
        assert_eq!(clamp_word_count(Some(2.5)), 3);
        assert_eq!(clamp_word_count(Some(f64::INFINITY)), 2);
    }

    #[test]
    fn registers_both_new_schemes_so_the_profile_can_offer_them() {
        assert!(NAMING_SCHEMES.contains(&"words"));
        assert!(NAMING_SCHEMES.contains(&"random-chars"));
        assert!(is_naming_scheme(Some("words")));
        assert!(is_naming_scheme(Some("random-chars")));
        assert!(!is_naming_scheme(Some("nonsense")));
        assert!(!is_naming_scheme(Some("")));
    }
}

mod domains {
    //! tests/unit/naming-domains.test.ts (userBaseUrl, URL builders; the
    //! config.appDomains cases are in config.rs).
    use modlight_server::media::urls::{delete_url, download_url, is_known_domain, public_url, raw_url, user_base_url};

    use super::{H, HOST, S, config};

    #[test]
    fn returns_the_deployment_default_when_no_preference_is_stored() {
        let c = config(&[("APP_URL", HOST), ("APP_DOMAINS", &format!("{H},{S}"))]);
        assert_eq!(user_base_url(&c, None), HOST);
        assert_eq!(user_base_url(&c, Some("")), HOST);
    }

    #[test]
    fn honours_a_configured_domain() {
        let c = config(&[("APP_URL", HOST), ("APP_DOMAINS", &format!("{H},{S}"))]);
        assert_eq!(user_base_url(&c, Some(H)), H);
        assert_eq!(user_base_url(&c, Some(&format!("{S}/"))), S);
    }

    #[test]
    fn refuses_an_origin_the_operator_never_configured() {
        // The value is echoed into ShareX configs and OG tags: a stored origin
        // outside the allow-list must never make it into a generated link.
        let c = config(&[("APP_URL", HOST), ("APP_DOMAINS", H)]);
        assert_eq!(user_base_url(&c, Some("https://evil.example.com")), HOST);
        assert!(!is_known_domain(&c, "https://evil.example.com"));
        assert!(is_known_domain(&c, H));
    }

    #[test]
    fn self_heals_when_a_domain_is_dropped_from_the_configuration() {
        let c = config(&[("APP_URL", HOST)]);
        // The account still points at h.example.com, but it is no longer served.
        assert_eq!(user_base_url(&c, Some(H)), HOST);
    }

    #[test]
    fn builds_every_public_link_on_the_supplied_origin() {
        assert_eq!(public_url("alicja", "funny-cat", "png", H), format!("{H}/alicja/funny-cat.png"));
        assert_eq!(raw_url("alicja", "funny-cat", "png", H), format!("{H}/alicja/funny-cat/raw.png"));
        assert_eq!(download_url("alicja", "funny-cat", H), format!("{H}/alicja/funny-cat/download"));
        assert_eq!(delete_url("alicja", "funny-cat", "tok", H), format!("{H}/alicja/funny-cat/delete/tok"));
        // Upstream's "defaults to APP_URL" case: the Rust builders take the base
        // explicitly; callers pass `config.app_url`.
        let c = config(&[("APP_URL", HOST), ("APP_DOMAINS", H)]);
        assert_eq!(public_url("alicja", "abc12", "png", &c.app_url), format!("{HOST}/alicja/abc12.png"));
        assert_eq!(raw_url("alicja", "abc12", "", &c.app_url), format!("{HOST}/alicja/abc12/raw"));
    }
}

mod media_urls_owner {
    //! tests/unit/media-urls-owner.test.ts
    use modlight_server::media::urls::{LinkOwner, media_urls};

    use super::{HOST, config};

    fn links(owner: LinkOwner, code: &str, ext: &str) -> (String, String) {
        let urls = media_urls(&config(&[("APP_URL", HOST)]), &owner, code, ext);
        (urls.url, urls.raw_url)
    }

    #[test]
    fn puts_the_tag_in_both_links_by_default() {
        let owner = LinkOwner { user_code: Some("ToqOse".into()), ..Default::default() };
        assert_eq!(
            links(owner, "abc12", "png"),
            (format!("{HOST}/ToqOse/abc12.png"), format!("{HOST}/ToqOse/abc12/raw.png"))
        );
    }

    #[test]
    fn drops_the_tag_from_both_links_when_the_account_opted_out() {
        let owner = LinkOwner { user_code: Some("ToqOse".into()), skip_tag_in_url: Some(true), ..Default::default() };
        assert_eq!(links(owner, "abc12", "png"), (format!("{HOST}/abc12.png"), format!("{HOST}/abc12/raw.png")));
    }

    #[test]
    fn treats_a_missing_skip_tag_in_url_as_off() {
        for skip in [None, Some(false)] {
            let owner = LinkOwner { user_code: Some("ToqOse".into()), skip_tag_in_url: skip, ..Default::default() };
            assert_eq!(links(owner, "abc12", "png").0, format!("{HOST}/ToqOse/abc12.png"));
        }
    }

    #[test]
    fn falls_back_to_a_placeholder_tag_rather_than_an_empty_segment() {
        assert_eq!(links(LinkOwner::default(), "abc12", "png").0, format!("{HOST}/u/abc12.png"));
    }

    #[test]
    fn handles_a_file_with_no_extension() {
        let owner = LinkOwner { user_code: Some("ToqOse".into()), ..Default::default() };
        assert_eq!(links(owner, "abc12", ""), (format!("{HOST}/ToqOse/abc12"), format!("{HOST}/ToqOse/abc12/raw")));
    }

    #[test]
    fn ignores_a_preferred_domain_the_operator_never_configured() {
        let owner = LinkOwner {
            user_code: Some("ToqOse".into()),
            preferred_domain: Some("https://evil.example".into()),
            ..Default::default()
        };
        assert_eq!(
            links(owner, "abc12", "png"),
            (format!("{HOST}/ToqOse/abc12.png"), format!("{HOST}/ToqOse/abc12/raw.png"))
        );
    }
}

mod background {
    //! tests/unit/background.test.ts
    use modlight_server::background::{
        BackgroundStyle, DIM_DEFAULT, DIM_MAX, DIM_MIN, DimInput, background_key, background_style, background_token,
        background_url, clamp_dim, is_accepted_background_type,
    };

    #[test]
    fn clamp_dim_keeps_sane_values() {
        assert_eq!(clamp_dim(DimInput::Number(0.0)), 0);
        assert_eq!(clamp_dim(DimInput::Number(55.0)), 55);
        assert_eq!(clamp_dim(DimInput::Number(95.0)), 95);
    }

    #[test]
    fn clamp_dim_clamps_out_of_range_values_instead_of_rejecting_them() {
        assert_eq!(clamp_dim(DimInput::Number(-40.0)), DIM_MIN as i32);
        assert_eq!(clamp_dim(DimInput::Number(1000.0)), DIM_MAX as i32);
    }

    #[test]
    fn clamp_dim_falls_back_to_the_default_for_anything_non_numeric() {
        assert_eq!(clamp_dim(DimInput::Absent), DIM_DEFAULT);
        assert_eq!(clamp_dim(DimInput::Text("abc")), DIM_DEFAULT);
        assert_eq!(clamp_dim(DimInput::Number(f64::NAN)), DIM_DEFAULT);
        assert_eq!(clamp_dim(DimInput::Number(f64::INFINITY)), DIM_DEFAULT);
    }

    #[test]
    fn clamp_dim_rounds_so_a_fractional_drag_never_reaches_css() {
        assert_eq!(clamp_dim(DimInput::Number(55.4)), 55);
        assert_eq!(clamp_dim(DimInput::Number(55.6)), 56);
    }

    #[test]
    fn round_trips_the_cache_busting_token() {
        let key = background_key("user_123", "deadbeef");
        assert_eq!(key, "backgrounds/user_123-deadbeef.webp");
        assert_eq!(background_token(Some(&key)), "deadbeef");
    }

    #[test]
    fn takes_the_last_dash_so_an_id_containing_dashes_still_resolves() {
        assert_eq!(background_token(Some(&background_key("a-b-c", "cafe"))), "cafe");
    }

    #[test]
    fn token_is_empty_for_anything_that_is_not_one_of_our_keys() {
        assert_eq!(background_token(None), "");
        assert_eq!(background_token(Some("nodash")), "");
    }

    #[test]
    fn url_is_null_unless_there_is_both_a_tag_and_a_key() {
        assert_eq!(background_url(None, Some("backgrounds/u-1.webp")), None);
        assert_eq!(background_url(Some("tag"), None), None);
        assert_eq!(background_url(None, None), None);
    }

    #[test]
    fn url_is_root_relative_and_escapes_the_tag() {
        assert_eq!(
            background_url(Some("mytag"), Some(&background_key("u1", "abc123"))).as_deref(),
            Some("/bg/mytag?v=abc123")
        );
        let url = background_url(Some("a\"b"), Some(&background_key("u1", "t"))).unwrap();
        assert!(!url.contains('"'));
        assert_eq!(url, "/bg/a%22b?v=t");
        assert_eq!(background_url(Some("tag"), Some("legacy.webp")).as_deref(), Some("/bg/tag"));
    }

    #[test]
    fn style_hands_css_a_fraction() {
        assert_eq!(
            background_style(Some("tag"), Some(&background_key("u1", "t")), Some(60)),
            Some(BackgroundStyle { url: "/bg/tag?v=t".into(), dim: 0.6 })
        );
        assert_eq!(background_style(Some("tag"), None, Some(60)), None);
        let style = background_style(Some("tag"), Some(&background_key("u1", "t")), None).unwrap();
        assert!((style.dim - f64::from(DIM_DEFAULT) / 100.0).abs() < 1e-9);
    }

    #[test]
    fn accepts_the_raster_formats_and_rejects_the_rest() {
        for t in ["image/png", "image/jpeg", "image/webp", "image/avif", "image/gif"] {
            assert!(is_accepted_background_type(Some(t)), "{t}");
        }
        assert!(is_accepted_background_type(Some("IMAGE/PNG")));
        assert!(is_accepted_background_type(Some("image/jpeg; charset=binary")));
        for t in [Some("image/svg+xml"), Some("text/html"), Some("application/octet-stream"), None, Some("")] {
            assert!(!is_accepted_background_type(t), "{t:?}");
        }
    }
}

mod avatar {
    //! tests/unit/avatar.test.ts (key, token and URL helpers)
    use modlight_server::avatar::{avatar_key, avatar_token, avatar_url, is_avatar_key};

    #[test]
    fn round_trips_the_cache_busting_token() {
        let key = avatar_key("user_123", "deadbeef");
        assert_eq!(key, "avatars/user_123-deadbeef.webp");
        assert_eq!(avatar_token(Some(&key)), "deadbeef");
        assert_eq!(avatar_token(Some(&avatar_key("a-b-c", "cafe"))), "cafe");
        assert_eq!(avatar_token(None), "");
        assert_eq!(avatar_token(Some("nodash")), "");
    }

    #[test]
    fn recognises_only_values_this_app_wrote() {
        assert!(is_avatar_key(Some(&avatar_key("u1", "t"))));
        assert!(!is_avatar_key(Some("backgrounds/u1-t.webp")));
        assert!(!is_avatar_key(Some("https://cdn.example.com/face.png")));
        assert!(!is_avatar_key(None));
        assert!(!is_avatar_key(Some("")));
    }

    #[test]
    fn url_is_root_relative_and_escapes_the_tag() {
        assert_eq!(
            avatar_url(Some("mytag"), Some(&avatar_key("u1", "abc123"))).as_deref(),
            Some("/avatar/mytag?v=abc123")
        );
        let url = avatar_url(Some("a\"b"), Some(&avatar_key("u1", "t"))).unwrap();
        assert!(!url.contains('"'));
        assert_eq!(url, "/avatar/a%22b?v=t");
    }

    #[test]
    fn passes_an_absolute_url_through_untouched() {
        assert_eq!(
            avatar_url(Some("tag"), Some("https://cdn.example.com/face.png")).as_deref(),
            Some("https://cdn.example.com/face.png")
        );
    }

    #[test]
    fn degrades_to_initials_rather_than_to_a_broken_image() {
        assert_eq!(avatar_url(Some("tag"), None), None);
        assert_eq!(avatar_url(Some("tag"), Some("")), None);
        assert_eq!(avatar_url(None, Some(&avatar_key("u1", "t"))), None);
        assert_eq!(avatar_url(Some("tag"), Some("some/other/object.bin")), None);
    }

    #[test]
    fn omits_v_when_the_key_carries_no_token() {
        assert_eq!(avatar_url(Some("tag"), Some("avatars/legacy.webp")).as_deref(), Some("/avatar/tag"));
    }
}

mod quota {
    //! tests/unit/quota.test.ts
    use modlight_server::quota::{UNLIMITED_QUOTA, format_quota_input, is_unlimited, parse_quota, quota_ratio};

    #[test]
    fn parses_the_common_suffixes_as_binary_multiples() {
        assert_eq!(parse_quota("1K"), Some(1024));
        assert_eq!(parse_quota("512M"), Some(512 * 1024 * 1024));
        assert_eq!(parse_quota("20G"), Some(20 * 1024 * 1024 * 1024));
        assert_eq!(parse_quota("1T"), Some(1024_i64.pow(4)));
    }

    #[test]
    fn accepts_the_long_and_iec_spellings_case_insensitively() {
        assert_eq!(parse_quota("512mb"), Some(512 * 1024 * 1024));
        assert_eq!(parse_quota("512MiB"), Some(512 * 1024 * 1024));
        assert_eq!(parse_quota("2 GB"), Some(2 * 1024 * 1024 * 1024));
    }

    #[test]
    fn treats_a_bare_number_as_bytes() {
        assert_eq!(parse_quota("1024"), Some(1024));
        assert_eq!(parse_quota("0"), Some(0));
    }

    #[test]
    fn accepts_a_comma_decimal() {
        assert_eq!(parse_quota("1,5G"), Some((1.5 * 1024f64.powi(3)).round() as i64));
        assert_eq!(parse_quota("1.5G"), Some((1.5 * 1024f64.powi(3)).round() as i64));
    }

    #[test]
    fn maps_every_no_limit_spelling_onto_the_sentinel() {
        for v in ["", "  ", "-1", "unlimited", "none", "\u{221e}"] {
            assert_eq!(parse_quota(v), Some(UNLIMITED_QUOTA), "{v:?}");
        }
    }

    #[test]
    fn returns_none_rather_than_guessing_on_junk() {
        for v in ["abc", "20 gigs", "12X", "1e3"] {
            assert_eq!(parse_quota(v), None, "{v}");
        }
    }

    #[test]
    fn refuses_sizes_that_stop_being_exact_js_integers() {
        assert_eq!(parse_quota("9007199254740993"), None);
        assert_eq!(parse_quota("1024P"), None);
    }

    #[test]
    fn format_round_trips_the_values_parse_produces() {
        for text in ["1K", "512M", "20G", "1T"] {
            assert_eq!(format_quota_input(parse_quota(text)), text);
        }
        assert_eq!(format_quota_input(Some(-1)), "");
        assert_eq!(format_quota_input(None), "");
        assert_eq!(format_quota_input(Some(1500)), "1500");
    }

    #[test]
    fn unlimited_and_ratio() {
        assert!(is_unlimited(Some(-1)));
        assert!(is_unlimited(None));
        assert!(!is_unlimited(Some(0)));
        assert!(!is_unlimited(Some(1024)));
        assert_eq!(quota_ratio(1024.0, Some(-1)), None);
        assert_eq!(quota_ratio(1024.0, None), None);
        assert_eq!(quota_ratio(512.0, Some(1024)), Some(0.5));
        assert_eq!(quota_ratio(4096.0, Some(1024)), Some(1.0));
        assert_eq!(quota_ratio(0.0, Some(1024)), Some(0.0));
    }
}

mod action_exports {
    //! tests/unit/server-action-exports.test.ts guards upstream against an
    //! export that vanishes at runtime (every export of a "use server" module
    //! is a live endpoint). The port's equivalent: every export of
    //! `app/actions/account.ts` that the SPA's generated stubs call is a
    //! registered action, and nothing else is registered under `account`.
    use serde_json::Value;

    #[test]
    fn every_account_stub_of_the_spa_reaches_a_registered_action() {
        let manifest =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/web/src/rpc/actions.manifest.json"))
                .expect("web/src/rpc/actions.manifest.json");
        let manifest: Vec<Value> = serde_json::from_str(&manifest).unwrap();
        let mut stubs: Vec<String> = manifest
            .iter()
            .filter(|a| a["module"] == "account")
            .map(|a| a["name"].as_str().unwrap().to_string())
            .collect();
        stubs.sort();
        let mut exports: Vec<String> =
            modlight_server::actions::account::EXPORTS.iter().map(|s| (*s).to_string()).collect();
        exports.sort();
        assert_eq!(stubs, exports);

        let mut reg = modlight_server::rpc::RpcRegistry::default();
        modlight_server::actions::register_all(&mut reg);
        let registered: Vec<String> =
            reg.names().into_iter().filter_map(|n| n.strip_prefix("account/").map(str::to_string)).collect();
        assert_eq!(registered, exports);
    }
}

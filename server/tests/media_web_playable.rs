//! tests/unit/web-playable.test.ts ("who is offered a web-playable copy",
//! "the menu and the route cannot come apart"; the scale filter, the ffmpeg
//! command and the pixel budget are tested next to av_convert and av_cost)
//! and tests/unit/video-playback.test.ts.

use modlight_server::db::types::JsDate;
use modlight_server::media::av_targets::{
    AvCost, AvSourceSize, AvTarget, AvYield, WEB_MAX_EDGE, av_option_for, av_output, av_targets_for, needs_web_rescue,
};
use modlight_server::media::convert_plan::{
    ConvertCandidate, ConvertMode, ConvertPlan, conversion_engine_for, plan_conversion,
};
use modlight_server::media::download_menu::{
    DownloadMenuOptions, DownloadSection, Engine, download_hint_key, download_label_key, download_options_for,
};
use modlight_server::media::mime::{playable_audio_type, playable_video_type, serve_decision};

const CAPTURE_ROW: AvSourceSize = AvSourceSize { width: Some(5056.0), height: Some(1924.0) };

fn size(w: f64, h: f64) -> AvSourceSize {
    AvSourceSize { width: Some(w), height: Some(h) }
}

fn names(o: &[modlight_server::media::av_targets::AvOption]) -> Vec<&'static str> {
    o.iter().map(|x| x.to.as_str()).collect()
}

#[test]
fn who_is_offered_a_web_playable_copy() {
    let options = av_targets_for(Some("video/mp4"), Some("mp4"), Some(403.0), Some(&CAPTURE_ROW));
    assert_eq!(names(&options)[0], "webmp4");
    assert_eq!(
        (options[0].to, options[0].cost, options[0].yields),
        (AvTarget::Webmp4, AvCost::Encode, AvYield::WebVideo)
    );
    for s in [size(1280.0, 720.0), size(1920.0, 1080.0)] {
        assert!(!names(&av_targets_for(Some("video/mp4"), Some("mp4"), Some(300.0), Some(&s))).contains(&"webmp4"));
    }
    assert!(!names(&av_targets_for(Some("video/mp4"), Some("mp4"), Some(403.0), None)).contains(&"webmp4"));
    let unmeasured = AvSourceSize { width: None, height: None };
    assert!(
        !names(&av_targets_for(Some("video/mp4"), Some("mp4"), Some(403.0), Some(&unmeasured))).contains(&"webmp4")
    );
    assert!(needs_web_rescue(Some(&size(1080.0, 2400.0))));
    assert!(needs_web_rescue(Some(&size(2400.0, 1080.0))));
    assert!(!needs_web_rescue(Some(&size(WEB_MAX_EDGE, WEB_MAX_EDGE))));
    assert!(needs_web_rescue(Some(&size(WEB_MAX_EDGE + 1.0, 100.0))));
    assert!(!needs_web_rescue(None));
    assert!(!needs_web_rescue(Some(&unmeasured)));
    let big = size(8000.0, 8000.0);
    for (mime, ext, frames) in [
        ("audio/mpeg", "mp3", None),
        ("image/gif", "gif", Some(128.0)),
        ("image/avif", "avif", None),
        ("image/png", "png", Some(1.0)),
    ] {
        assert!(!names(&av_targets_for(Some(mime), Some(ext), frames, Some(&big))).contains(&"webmp4"));
    }
    assert_eq!(av_output(AvTarget::Webmp4), ("video/mp4", "mp4"));
}

#[test]
fn the_menu_and_the_route_cannot_come_apart() {
    let mp4 = |s: Option<&AvSourceSize>| av_option_for(Some("video/mp4"), Some("mp4"), Some(403.0), Some("webmp4"), s);
    assert!(mp4(Some(&CAPTURE_ROW)).is_some());
    assert!(mp4(Some(&size(1920.0, 1080.0))).is_none());
    assert!(mp4(None).is_none());

    let menu = |can_encode| {
        download_options_for(
            Some("video/mp4"),
            Some("mp4"),
            Some(403.0),
            DownloadMenuOptions { can_encode, size: CAPTURE_ROW },
        )
    };
    assert!(menu(true).iter().any(|o| o.to == "webmp4"));
    assert!(!menu(false).iter().any(|o| o.to == "webmp4"));
    assert_eq!(download_label_key("webmp4"), Some("convert.webPlayableLabel"));
    assert_eq!(download_label_key("mp4"), None);
    let option = menu(true).into_iter().find(|o| o.to == "webmp4").unwrap();
    assert_eq!(option.section, DownloadSection::Main);
    assert_eq!(download_hint_key(&option), Some("convert.webPlayable"));

    assert_eq!(
        conversion_engine_for(Some("video/mp4"), Some("mp4"), Some(403.0), Some("webmp4"), Some(&CAPTURE_ROW)),
        Some(Engine::Av)
    );
    assert_eq!(conversion_engine_for(Some("video/mp4"), Some("mp4"), Some(403.0), Some("webmp4"), None), None);
    let row = ConvertCandidate {
        id: "u1".into(),
        ext: Some("mp4".into()),
        mime_type: Some("video/mp4".into()),
        frames: Some(403.0),
        width: Some(5056.0),
        height: Some(1924.0),
        size: 2_179_538.0,
        kind: "file".into(),
        deleted_at: None,
        expires_at: None,
    };
    assert_eq!(
        plan_conversion(&row, "webmp4", ConvertMode::Replace, JsDate::now()),
        ConvertPlan::Ok { engine: Engine::Av, ext: "mp4", mime: "video/mp4" }
    );
}

#[test]
fn video_containers_a_browser_can_actually_decode() {
    for (mime, ext) in [("video/quicktime", "mov"), ("video/x-m4v", "m4v"), ("video/3gpp", "3gp")] {
        assert_eq!(playable_video_type(Some(mime), Some(ext)), Some("video/mp4"));
    }
    assert_eq!(playable_video_type(Some("video/mp4"), Some("mp4")), Some("video/mp4"));
    assert_eq!(playable_video_type(Some("video/webm"), Some("webm")), Some("video/webm"));
    assert_eq!(playable_video_type(Some("video/ogg"), Some("ogv")), Some("video/ogg"));
    for (mime, ext) in [
        ("video/x-matroska", "mkv"),
        ("video/x-msvideo", "avi"),
        ("video/x-ms-wmv", "wmv"),
        ("video/x-flv", "flv"),
        ("video/mpeg", "mpg"),
        ("video/mp2t", "ts"),
    ] {
        assert_eq!(playable_video_type(Some(mime), Some(ext)), None, "{mime}");
    }
    assert_eq!(playable_video_type(Some("application/octet-stream"), Some("mov")), Some("video/mp4"));
    assert_eq!(playable_video_type(Some("application/octet-stream"), Some("mp4")), Some("video/mp4"));
    assert_eq!(playable_video_type(None, Some("webm")), Some("video/webm"));
    assert_eq!(playable_video_type(Some("application/octet-stream"), Some("mkv")), None);
    assert_eq!(playable_video_type(Some("video/x-matroska"), Some("mp4")), None);

    let mov = serve_decision(Some("video/quicktime"), Some("mov"));
    assert_eq!((mov.disposition, mov.content_type.as_str()), ("inline", "video/mp4"));
    assert_eq!(serve_decision(Some("video/x-matroska"), Some("mkv")).disposition, "attachment");
    let png = serve_decision(Some("image/png"), Some("png"));
    assert_eq!((png.disposition, png.content_type.as_str()), ("inline", "image/png"));
    let html = serve_decision(Some("text/html"), Some("html"));
    assert_eq!((html.disposition, html.content_type.as_str()), ("inline", "text/plain; charset=utf-8"));

    assert_eq!(playable_audio_type(Some("audio/x-m4a"), Some("m4a")), Some("audio/mp4"));
    assert_eq!(playable_audio_type(Some("application/octet-stream"), Some("mp3")), Some("audio/mpeg"));
    assert_eq!(playable_audio_type(Some("audio/x-ms-wma"), Some("wma")), None);
}

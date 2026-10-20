//! Developer tool for the media parity checks (tests/parity_media): runs this
//! crate's media-processing functions on local files and writes what they
//! produced, so the result can be compared with what the reference server
//! answered for the same row and the same bytes.
//!
//! ```text
//! cargo run --example media_probe -- card <category>-<TOKEN>.png <out.png>
//! cargo run --example media_probe -- batch <manifest.json> <out-dir>
//! ```
//!
//! `batch` reads `{"cases": [{"name", "row", "op", ...}]}` where `row` is an
//! `uploads` row (camelCase, as the reference database has it) whose
//! `storageKey` exists under STORAGE_LOCAL_PATH, and `op` is one of
//! `convert` (`to`, `check`, `privileged`, `t`, `d`), `motion` (`w`, `h`,
//! `fit`), `timeline.vtt`, `timeline.jpg`, `poster`. For each case it writes
//! `<out>/<name>/meta.json` (status, headers) and `body.bin`.
//!
//! `convert` follows the order of the upstream /convert route (av engine
//! first, then the image branch: admission, size, probe, budget, encode)
//! without its view counting and cache, so what is compared is the work.

use std::collections::BTreeMap;
use std::path::Path;

use axum::body::Body;
use axum::response::Response;
use bytes::Bytes;
use http_body_util::BodyExt;
use modlight_server::db::models::Upload;
use modlight_server::media::abort::AbortController;
use modlight_server::media::av_action::{AvConvertRequest, av_convert};
use modlight_server::media::convert::{
    ConvertOptions, conversion_budget_for, convert_image_buffer, convert_option_for,
};
use modlight_server::media::motion::{MotionFit, MotionOptions, motion_preview};
use modlight_server::media::poster::video_poster;
use modlight_server::media::probe::probe_image;
use modlight_server::media::timeline::{timeline_sheet, timeline_track};
use modlight_server::media::transform_cost::{CONVERT_TIMEOUT_MS, MAX_TRANSFORM_BYTES};
use modlight_server::state::AppState;
use serde_json::{Value, json};

struct Out {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Bytes,
}

fn out(status: u16, headers: &[(&str, &str)], body: impl Into<Bytes>) -> Out {
    Out { status, headers: headers.iter().map(|(k, v)| (k.to_lowercase(), v.to_string())).collect(), body: body.into() }
}

async fn from_response(res: Response<Body>) -> anyhow::Result<Out> {
    let (parts, body) = res.into_parts();
    let body = body.collect().await.map_err(|e| anyhow::anyhow!("{e}"))?.to_bytes();
    let headers = parts.headers.iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect();
    Ok(Out { status: parts.status.as_u16(), headers, body })
}

fn image_refusal(status: u16, reason: &str) -> Out {
    out(status, &[("content-type", "text/plain; charset=utf-8"), ("x-convert-refused", reason)], Bytes::new())
}

async fn convert(state: &AppState, media: &Upload, case: &Value) -> anyhow::Result<Out> {
    let to = case["to"].as_str().map(str::to_string);
    let check = case["check"].as_bool().unwrap_or(false);
    let privileged = case["privileged"].as_bool().unwrap_or(true);
    let req = AvConvertRequest {
        media: media.clone(),
        privileged,
        to: to.clone(),
        offset_seconds: case["t"].as_f64(),
        clip_seconds: case["d"].as_f64(),
        signal: None,
        admit: None,
        dry_run: check,
    };
    match av_convert(state, req).await {
        Ok(Some(res)) => return from_response(res).await,
        Ok(None) => {}
        Err(e) => return Ok(out(599, &[("x-error", &e.to_string())], Bytes::new())),
    }
    let Some(option) =
        convert_option_for(Some(&media.mime_type), Some(&media.ext), media.frames.map(f64::from), to.as_deref())
    else {
        return Ok(image_refusal(415, "wrong-target"));
    };
    if media.size as u64 > MAX_TRANSFORM_BYTES {
        return Ok(image_refusal(413, "too-large"));
    }
    let src = state.storage.get()?.read_buffer(&media.storage_key).await?;
    let Some(probe) = probe_image(src).await else {
        return Ok(image_refusal(415, "unreadable"));
    };
    let budget = conversion_budget_for(&probe, option.to);
    if !budget.ok {
        return Ok(image_refusal(413, "too-many-pixels"));
    }
    if check {
        return Ok(out(204, &[], Bytes::new()));
    }
    let converted = convert_image_buffer(
        probe.input.clone(),
        option.to,
        ConvertOptions { animated: budget.animated, timeout_ms: Some(CONVERT_TIMEOUT_MS), signal: None },
    )
    .await?;
    Ok(out(200, &[("content-type", converted.mime)], converted.buffer))
}

async fn run_case(state: &AppState, case: &Value) -> anyhow::Result<Out> {
    let media: Upload = serde_json::from_value(case["row"].clone())?;
    let signal = AbortController::new().signal();
    match case["op"].as_str().unwrap_or("") {
        "convert" => convert(state, &media, case).await,
        "motion" => {
            let opts = MotionOptions {
                width: case["w"].as_u64().map(|v| v as u32),
                height: case["h"].as_u64().map(|v| v as u32),
                fit: if case["fit"].as_str() == Some("inside") { MotionFit::Inside } else { MotionFit::Cover },
            };
            Ok(match motion_preview(state, &media, opts, signal).await {
                Ok(Ok((buffer, frames))) => {
                    out(200, &[("content-type", "image/webp"), ("x-motion-frames", &frames.to_string())], buffer)
                }
                Ok(Err(refusal)) => {
                    let status = if refusal.as_str() == "too-large" { 413 } else { 415 };
                    out(status, &[("x-motion-refused", refusal.as_str())], Bytes::new())
                }
                Err(e) => out(599, &[("x-error", &e.to_string())], Bytes::new()),
            })
        }
        "timeline.vtt" => Ok(match timeline_track(state, &media, signal).await {
            Ok(Ok(vtt)) => out(200, &[("content-type", "text/vtt; charset=utf-8")], vtt),
            Ok(Err(r)) => out(415, &[("x-timeline-refused", r.as_str())], Bytes::new()),
            Err(e) => out(599, &[("x-error", &e.to_string())], Bytes::new()),
        }),
        "timeline.jpg" => Ok(match timeline_sheet(state, &media, signal).await {
            Ok(Ok(jpg)) => out(200, &[("content-type", "image/jpeg")], jpg),
            Ok(Err(r)) => out(415, &[("x-timeline-refused", r.as_str())], Bytes::new()),
            Err(e) => out(599, &[("x-error", &e.to_string())], Bytes::new()),
        }),
        "poster" => Ok(match video_poster(state, &media, signal).await {
            Some(jpg) => out(200, &[("content-type", "image/jpeg")], jpg),
            None => out(415, &[], Bytes::new()),
        }),
        other => anyhow::bail!("unknown op {other}"),
    }
}

fn safe_name(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' }).collect()
}

async fn batch(manifest: &Path, dir: &Path) -> anyhow::Result<()> {
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest)?)?;
    let config = modlight_server::config::Config::from_env()?;
    let pool = modlight_server::db::connect_lazy("postgresql://nobody@127.0.0.1:1/none")?;
    let state = AppState::new(config, modlight_server::paths::Paths::resolve(), pool)?;
    for case in manifest["cases"].as_array().cloned().unwrap_or_default() {
        let name = case["name"].as_str().unwrap_or("case").to_string();
        // A marker in the ffmpeg argument log (the parity shim's), so every
        // logged invocation can be attributed to its case.
        if let Ok(log) = std::env::var("MEDIA_PROBE_ARGLOG") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
                let _ = writeln!(f, "#case\x1f{name}");
            }
        }
        let started = std::time::Instant::now();
        let result = match run_case(&state, &case).await {
            Ok(o) => o,
            Err(e) => out(598, &[("x-error", &format!("{e:#}"))], Bytes::new()),
        };
        let case_dir = dir.join(safe_name(&name));
        std::fs::create_dir_all(&case_dir)?;
        std::fs::write(case_dir.join("body.bin"), &result.body)?;
        let meta = json!({
            "name": name,
            "status": result.status,
            "headers": result.headers,
            "size": result.body.len(),
            "ms": started.elapsed().as_millis() as u64,
        });
        std::fs::write(case_dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?)?;
        println!("{:>4} {:>6} ms  {name}", result.status, started.elapsed().as_millis());
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rt = tokio::runtime::Runtime::new()?;
    match args.first().map(String::as_str) {
        Some("card") => {
            let choice = modlight_server::media::card::parse_card_file(&args[1])
                .ok_or_else(|| anyhow::anyhow!("not a renderable card: {}", args[1]))?;
            let png =
                rt.block_on(modlight_server::media::card_render::render_card(&choice)).map_err(anyhow::Error::msg)?;
            std::fs::write(&args[2], &png)?;
        }
        Some("batch") => rt.block_on(batch(Path::new(&args[1]), Path::new(&args[2])))?,
        other => anyhow::bail!("unknown command {other:?}"),
    }
    Ok(())
}

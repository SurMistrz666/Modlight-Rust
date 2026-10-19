//! Developer tool for the media parity checks (tests/parity_media): runs one
//! media-processing function of this crate on local files and writes what it
//! produced, so the result can be compared with the reference server's.
//!
//! cargo run --example media_probe -- card <category>-<TOKEN>.png <out.png>
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
        other => anyhow::bail!("unknown command {other:?}"),
    }
    Ok(())
}

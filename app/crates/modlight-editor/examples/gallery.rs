//! Renders one of every annotation onto a test picture: `cargo run -p modlight-editor
//! --example gallery -- out.png`. A development aid for eyeballing the renderer; the same
//! picture is the golden of `tests/render.rs`.

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "gallery.png".into());
    modlight_editor::qa::gallery().save(&out).unwrap();
    println!("{out}");
}

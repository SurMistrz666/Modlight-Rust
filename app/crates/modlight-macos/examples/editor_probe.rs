//! Prints what the image editor's system helpers return here (cursors, the desktop
//! picture). Opens no window and asks for no permission.

fn main() {
    for name in [
        "Arrow", "IBeam", "Hand", "Cross", "SizeNWSE", "SizeNS", "No",
    ] {
        match modlight_macos::editor::cursor_image(name) {
            Some(image) => println!("cursor {name}: {}x{}", image.width(), image.height()),
            None => println!("cursor {name}: none"),
        }
    }
    match modlight_macos::editor::desktop_wallpaper() {
        Some((path, options)) => {
            println!("wallpaper: {} {:?}", path.display(), options);
            match modlight_macos::editor::load_image(&path) {
                Some(image) => println!("decoded: {}x{}", image.width(), image.height()),
                None => println!("decoded: none"),
            }
        }
        None => println!("wallpaper: none"),
    }
    if let Some(out) = std::env::args().nth(1)
        && let Some(image) = modlight_macos::editor::cursor_image(
            &std::env::args().nth(2).unwrap_or_else(|| "Arrow".into()),
        )
    {
        let _ = image.save(out);
    }
}

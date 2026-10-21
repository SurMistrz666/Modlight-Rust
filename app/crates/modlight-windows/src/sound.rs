//! The notification sounds (`Helpers.PlaySoundAsync` / `PlaySoundSync`): upstream's own
//! WAV files, played with `PlaySound`, as upstream's `SoundPlayer` does.

/// Play WAV bytes, without waiting.
pub fn play_bytes(bytes: &[u8]) -> bool {
    let _ = bytes;
    false
}

/// Play a sound file the person chose, without waiting.
pub fn play_file(path: &std::path::Path) -> bool {
    let _ = path;
    false
}

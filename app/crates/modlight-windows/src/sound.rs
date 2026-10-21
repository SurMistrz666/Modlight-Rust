//! The notification sounds (`Helpers.PlaySoundAsync` / `PlaySoundSync`): upstream's own
//! WAV files, played with `PlaySound`, as upstream's `SoundPlayer` does.
//!
//! One sound plays at a time, as with `PlaySound` everywhere: a new one stops the one
//! before. A sound played from memory reads its bytes while it plays, so the last one is
//! kept here until the next replaces it, and the playing sound is stopped first.

use std::sync::Mutex;

use windows_sys::Win32::Media::Audio::{
    PlaySoundW, SND_ASYNC, SND_FILENAME, SND_MEMORY, SND_NODEFAULT,
};

use crate::wide::wide;

/// The bytes of the sound playing from memory.
static PLAYING: Mutex<Option<Box<[u8]>>> = Mutex::new(None);

/// Whether `bytes` start as a WAV file does (`RIFF....WAVE`).
fn is_wav(bytes: &[u8]) -> bool {
    bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE"
}

/// Play WAV bytes, without waiting.
pub fn play_bytes(bytes: &[u8]) -> bool {
    if !is_wav(bytes) {
        return false;
    }
    let mut playing = PLAYING.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: stops this process's sound, so the buffer it reads can be replaced.
    unsafe { PlaySoundW(std::ptr::null(), std::ptr::null_mut(), 0) };
    let buffer: Box<[u8]> = bytes.into();
    // SAFETY: with SND_MEMORY the first argument is the WAV image; it is kept alive in
    // PLAYING until a later call has stopped it.
    let started = unsafe {
        PlaySoundW(
            buffer.as_ptr().cast(),
            std::ptr::null_mut(),
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        )
    } != 0;
    *playing = Some(buffer);
    started
}

/// Play a sound file the person chose, without waiting. Nothing plays (not even the
/// system's default sound) when it cannot be read.
pub fn play_file(path: &std::path::Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let name = wide(path);
    // The lock keeps a memory sound from being started and stopped halfway through this.
    let _playing = PLAYING.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: `name` is NUL-terminated; PlaySound copies it before returning.
    unsafe {
        PlaySoundW(
            name.as_ptr(),
            std::ptr::null_mut(),
            SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
        ) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A short WAV: 16-bit mono at 8 kHz, a tenth of a second of a soft tone
    /// (`volume` 0 is silence).
    fn tone_at(volume: f32) -> Vec<u8> {
        let samples: Vec<i16> = (0..800)
            .map(|i| ((i as f32 * 0.3).sin() * volume) as i16)
            .collect();
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        wav
    }

    #[test]
    fn what_is_not_a_wav_is_not_played() {
        assert!(!play_bytes(b""));
        assert!(!play_bytes(b"ID3\x04 not a wave file"));
        assert!(!play_file(std::path::Path::new(
            r"C:\no such folder of modlight\x.wav"
        )));
        assert!(is_wav(&tone_at(0.0)));
    }

    fn plays(volume: f32) {
        assert!(play_bytes(&tone_at(volume)));
        // A second sound replaces the first while it plays.
        assert!(play_bytes(&tone_at(volume)));
        std::thread::sleep(std::time::Duration::from_millis(300));
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("tone.wav");
        std::fs::write(&file, tone_at(volume)).unwrap();
        assert!(play_file(&file));
        std::thread::sleep(std::time::Duration::from_millis(300));
    }

    /// Needs an audio device (silence, nothing is heard): run by hand.
    #[test]
    #[ignore]
    fn silence_plays() {
        plays(0.0);
    }

    /// Audible: run by hand.
    #[test]
    #[ignore]
    fn a_tone_plays() {
        plays(800.0);
    }
}

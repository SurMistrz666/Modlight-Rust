//! The notification sounds (`Helpers.PlaySoundAsync` / `PlaySoundSync`): upstream's own
//! WAV files, played with NSSound. Playing a sound needs no permission.

#[cfg(target_os = "macos")]
use std::cell::RefCell;

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::AllocAnyThread;
#[cfg(target_os = "macos")]
use objc2_app_kit::NSSound;
#[cfg(target_os = "macos")]
use objc2_foundation::{NSData, NSString};

#[cfg(target_os = "macos")]
thread_local! {
    /// Sounds stay alive while they play; finished ones are dropped on the next call.
    static PLAYING: RefCell<Vec<Retained<NSSound>>> = const { RefCell::new(Vec::new()) };
}

#[cfg(target_os = "macos")]
fn keep(sound: Retained<NSSound>) {
    PLAYING.with(|playing| {
        let mut playing = playing.borrow_mut();
        playing.retain(|s| s.isPlaying());
        playing.push(sound);
    });
}

/// Play WAV bytes. Call on the main thread.
pub fn play_bytes(bytes: &[u8]) -> bool {
    #[cfg(target_os = "macos")]
    {
        let data = NSData::with_bytes(bytes);
        let Some(sound) = NSSound::initWithData(NSSound::alloc(), &data) else {
            return false;
        };
        let started = sound.play();
        keep(sound);
        started
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = bytes;
        false
    }
}

/// Play a sound file the person chose. Call on the main thread.
pub fn play_file(path: &std::path::Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        let path = NSString::from_str(&path.to_string_lossy());
        let Some(sound) = NSSound::initWithContentsOfFile_byReference(NSSound::alloc(), &path, true)
        else {
            return false;
        };
        let started = sound.play();
        keep(sound);
        started
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

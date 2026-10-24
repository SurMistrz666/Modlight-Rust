//! Encoded preview bytes kept for the session (`CloudThumbnailCache.cs`,
//! `CloudMotionCache.cs`), so scrolling back or re-opening a folder does not ask again.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

type Entries = (HashMap<String, Arc<Vec<u8>>>, VecDeque<String>);

/// Still thumbnails by file code: first in, first out, at most 400.
#[derive(Clone, Default)]
pub struct ThumbnailCache {
    inner: Arc<Mutex<Entries>>,
}

impl ThumbnailCache {
    pub const CAPACITY: usize = 400;

    pub fn get(&self, code: &str) -> Option<Arc<Vec<u8>>> {
        if code.is_empty() {
            return None;
        }
        self.inner.lock().unwrap().0.get(code).cloned()
    }

    pub fn put(&self, code: &str, bytes: Arc<Vec<u8>>) {
        if code.is_empty() {
            return;
        }
        let mut guard = self.inner.lock().unwrap();
        let (entries, order) = &mut *guard;
        if entries.contains_key(code) {
            return;
        }
        entries.insert(code.to_string(), bytes);
        order.push_back(code.to_string());
        while order.len() > Self::CAPACITY {
            if let Some(oldest) = order.pop_front() {
                entries.remove(&oldest);
            }
        }
    }

    /// A file's code changed from `old` to `new` (a custom URL). What is held under
    /// `old` is that file's picture and goes with it; whatever was held under `new`
    /// belonged to some other file that had the code before, and is dropped. The old
    /// code is free for another file to take, which must not find this picture there.
    pub fn rename(&self, old: &str, new: &str) {
        if old.is_empty() || old == new {
            return;
        }
        let mut guard = self.inner.lock().unwrap();
        let (entries, order) = &mut *guard;
        let moving = entries.remove(old);
        order.retain(|c| c != old);
        if new.is_empty() {
            return;
        }
        entries.remove(new);
        order.retain(|c| c != new);
        if let Some(bytes) = moving {
            entries.insert(new.to_string(), bytes);
            order.push_back(new.to_string());
        }
    }
}

/// Animated previews by file code, least recently used out first, at most 16 MiB of
/// encoded bytes; plus the codes the server said will NEVER have one.
#[derive(Clone, Default)]
pub struct MotionCache {
    inner: Arc<Mutex<MotionInner>>,
}

#[derive(Default)]
struct MotionInner {
    entries: HashMap<String, Arc<Vec<u8>>>,
    /// Most recent first.
    order: VecDeque<String>,
    held: usize,
    refused: HashSet<String>,
}

impl MotionCache {
    pub const MAX_BYTES: usize = 16 * 1024 * 1024;

    pub fn get(&self, code: &str) -> Option<Arc<Vec<u8>>> {
        if code.is_empty() {
            return None;
        }
        let mut inner = self.inner.lock().unwrap();
        let bytes = inner.entries.get(code).cloned()?;
        if let Some(position) = inner.order.iter().position(|c| c == code) {
            let value = inner.order.remove(position).unwrap();
            inner.order.push_front(value);
        }
        Some(bytes)
    }

    pub fn put(&self, code: &str, bytes: Arc<Vec<u8>>) {
        if code.is_empty() {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        if inner.entries.contains_key(code) {
            return;
        }
        inner.held += bytes.len();
        inner.entries.insert(code.to_string(), bytes);
        inner.order.push_front(code.to_string());
        while inner.held > Self::MAX_BYTES && inner.order.len() > 1 {
            if let Some(oldest) = inner.order.pop_back()
                && let Some(evicted) = inner.entries.remove(&oldest)
            {
                inner.held -= evicted.len();
            }
        }
    }

    /// The server said this file will never produce a moving preview.
    pub fn refuse(&self, code: &str) {
        if !code.is_empty() {
            self.inner.lock().unwrap().refused.insert(code.to_string());
        }
    }

    pub fn is_refused(&self, code: &str) -> bool {
        !code.is_empty() && self.inner.lock().unwrap().refused.contains(code)
    }

    /// A file's code changed from `old` to `new`: its preview and its refusal go with
    /// it, and what was filed under `new` for another file is dropped
    /// ([`ThumbnailCache::rename`]).
    pub fn rename(&self, old: &str, new: &str) {
        if old.is_empty() || old == new {
            return;
        }
        let mut inner = self.inner.lock().unwrap();
        let moving = inner.entries.remove(old);
        inner.order.retain(|c| c != old);
        let refused = inner.refused.remove(old);
        if let Some(bytes) = &moving {
            inner.held -= bytes.len();
        }
        if new.is_empty() {
            return;
        }
        if let Some(stale) = inner.entries.remove(new) {
            inner.held -= stale.len();
        }
        inner.order.retain(|c| c != new);
        inner.refused.remove(new);
        if refused {
            inner.refused.insert(new.to_string());
        }
        if let Some(bytes) = moving {
            inner.held += bytes.len();
            inner.entries.insert(new.to_string(), bytes);
            inner.order.push_front(new.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_thumbnail_cache_keeps_the_newest_400() {
        let cache = ThumbnailCache::default();
        for i in 0..401 {
            cache.put(&format!("c{i}"), Arc::new(vec![1]));
        }
        assert!(cache.get("c0").is_none());
        assert!(cache.get("c400").is_some());
        assert!(cache.get("").is_none());
    }

    #[test]
    fn the_motion_cache_is_bounded_by_bytes() {
        let cache = MotionCache::default();
        let big = Arc::new(vec![0u8; 6 * 1024 * 1024]);
        cache.put("a", big.clone());
        cache.put("b", big.clone());
        assert!(cache.get("a").is_some(), "touching a makes b the oldest");
        cache.put("c", big);
        assert!(cache.get("b").is_none());
        assert!(cache.get("a").is_some() && cache.get("c").is_some());
        cache.refuse("x");
        assert!(cache.is_refused("x") && !cache.is_refused("a"));
    }

    #[test]
    fn a_renamed_code_takes_its_bytes_and_frees_the_old_one() {
        let stills = ThumbnailCache::default();
        stills.put("old", Arc::new(vec![1]));
        stills.put("taken", Arc::new(vec![9]));
        stills.rename("old", "taken");
        assert!(stills.get("old").is_none());
        assert_eq!(stills.get("taken"), Some(Arc::new(vec![1])));
        stills.rename("absent", "taken");
        assert!(stills.get("taken").is_none(), "another file's picture is dropped");
        for i in 0..400 {
            stills.put(&format!("c{i}"), Arc::new(vec![0]));
        }
        assert!(stills.get("c0").is_some(), "no order entry was left behind");

        let motion = MotionCache::default();
        motion.put("old", Arc::new(vec![0u8; 10]));
        motion.refuse("never");
        motion.rename("old", "new");
        motion.rename("never", "still-never");
        assert!(motion.get("old").is_none());
        assert_eq!(motion.get("new").map(|b| b.len()), Some(10));
        assert!(!motion.is_refused("never") && motion.is_refused("still-never"));
        assert_eq!(motion.inner.lock().unwrap().held, 10);
        motion.rename("other", "new");
        assert!(motion.get("new").is_none());
        assert_eq!(motion.inner.lock().unwrap().held, 0);
    }
}

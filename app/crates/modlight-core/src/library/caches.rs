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
}

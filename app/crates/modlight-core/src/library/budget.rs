//! Memory ceilings for decoded pictures (`ThumbnailBudget.cs`, `MotionFrameBudget.cs`).

use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;

/// A least-recently-used ledger of decoded thumbnails, in bytes.
///
/// The tiles on screen are pinned and never evicted; [`ThumbnailBudget::add`] returns
/// what the caller should release to get back under the ceiling.
pub struct ThumbnailBudget<T: Eq + Hash + Clone> {
    capacity_bytes: u64,
    /// Most recent first.
    order: VecDeque<T>,
    costs: HashMap<T, u64>,
    pinned: HashSet<T>,
    bytes: u64,
}

impl<T: Eq + Hash + Clone> ThumbnailBudget<T> {
    pub fn new(capacity_bytes: u64) -> Self {
        ThumbnailBudget {
            capacity_bytes,
            order: VecDeque::new(),
            costs: HashMap::new(),
            pinned: HashSet::new(),
            bytes: 0,
        }
    }

    pub fn count(&self) -> usize {
        self.costs.len()
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Record a decoded picture. Returns the items evicted to stay within the budget.
    pub fn add(&mut self, item: T, cost: u64) -> Vec<T> {
        if let Some(existing) = self.costs.get_mut(&item) {
            self.bytes = self.bytes - *existing + cost;
            *existing = cost;
            self.move_to_front(&item);
        } else {
            self.costs.insert(item.clone(), cost);
            self.order.push_front(item);
            self.bytes += cost;
        }
        self.trim()
    }

    /// Mark as recently used.
    pub fn touch(&mut self, item: &T) {
        if self.costs.contains_key(item) {
            self.move_to_front(item);
        }
    }

    /// The items on screen right now; they cannot be evicted until the next call.
    pub fn pin(&mut self, items: &[T]) {
        self.pinned = items
            .iter()
            .filter(|i| self.costs.contains_key(*i))
            .cloned()
            .collect();
    }

    pub fn forget(&mut self, item: &T) {
        if let Some(cost) = self.costs.remove(item) {
            self.order.retain(|i| i != item);
            self.pinned.remove(item);
            self.bytes -= cost;
        }
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.costs.clear();
        self.pinned.clear();
        self.bytes = 0;
    }

    fn move_to_front(&mut self, item: &T) {
        if let Some(position) = self.order.iter().position(|i| i == item) {
            let value = self.order.remove(position).unwrap();
            self.order.push_front(value);
        }
    }

    /// Evict from the least recent end, never the most recent and never a pinned one.
    fn trim(&mut self) -> Vec<T> {
        let mut released = Vec::new();
        let mut index = self.order.len();
        while self.bytes > self.capacity_bytes && index > 1 {
            index -= 1;
            let item = self.order[index].clone();
            if self.pinned.contains(&item) {
                continue;
            }
            self.order.remove(index);
            if let Some(cost) = self.costs.remove(&item) {
                self.bytes -= cost;
            }
            released.push(item);
        }
        released
    }
}

/// How many frames of an animated preview to keep, and at what size.
pub mod motion {
    /// The tile's width at 2x (180 px card × 1.6): no reason to decode wider.
    pub const MAX_WIDTH: u32 = 288;
    /// The ceiling for one moving tile's decoded frames.
    pub const MAX_BYTES: u64 = 10 * 1024 * 1024;
    /// A second fence that holds whatever the server sends.
    pub const MAX_FRAMES: u32 = 60;
    /// Fewer than two frames do not move.
    pub const MIN_FRAMES: u32 = 2;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Plan {
        pub width: u32,
        pub height: u32,
        pub frames: u32,
        pub bytes: u64,
    }

    /// The decode plan for a `canvas_width` × `canvas_height` preview with `available`
    /// frames: never enlarged, never more than the byte ceiling allows (but at least two
    /// frames), never more frames than the artefact has.
    pub fn plan(canvas_width: i64, canvas_height: i64, available: i64) -> Plan {
        if canvas_width <= 0 || canvas_height <= 0 || available <= 0 {
            return Plan {
                width: 0,
                height: 0,
                frames: 0,
                bytes: 0,
            };
        }
        let width = (MAX_WIDTH as i64).min(canvas_width);
        let height =
            1.max((canvas_height as f64 * width as f64 / canvas_width as f64).round() as i64);
        let frame_bytes = 1.max(width * height * 4) as u64;
        let affordable = (MAX_BYTES / frame_bytes).min(i32::MAX as u64) as i64;
        let mut frames = available
            .min(MAX_FRAMES as i64)
            .min(affordable.max(MIN_FRAMES as i64));
        frames = frames.min(available);
        Plan {
            width: width as u32,
            height: height as u32,
            frames: frames as u32,
            bytes: frame_bytes * frames as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::motion::{MAX_BYTES, MAX_FRAMES, plan};
    use super::*;

    fn mib(bytes: u64) -> String {
        format!("{:.2}", bytes as f64 / 1_048_576.0)
    }

    // ModlightLibraryTests.TheMotionBudgetHoldsTheClipsProductionSends
    #[test]
    fn the_motion_budget_holds_the_clips_production_sends() {
        let square = plan(480, 480, 30);
        assert!(
            square.width == 288 && square.height == 288,
            "a 480x480 preview decodes to 288x288"
        );
        assert_eq!(square.frames, 30, "and holds the whole thirty-frame clip");
        assert_eq!(
            mib(square.bytes),
            "9.49",
            "at 9.49 MiB, the figure measured on production"
        );

        let tall = plan(288, 480, 30);
        assert!(
            tall.width == 288 && tall.height == 480,
            "a 9:16 preview decodes to 288x480"
        );
        assert_eq!(tall.frames, 18, "and keeps eighteen frames of the thirty");
        assert!(
            tall.bytes <= MAX_BYTES,
            "which is inside the ceiling, not over it"
        );

        let portrait = plan(352, 480, 30);
        assert!(
            portrait.width == 288 && portrait.height == 393,
            "a 3:4 preview decodes to 288x393"
        );
        assert_eq!(portrait.frames, 23, "and keeps twenty-three of the thirty");

        let whole = [
            (480, 480),
            (480, 382),
            (480, 330),
            (480, 304),
            (480, 268),
            (400, 300),
        ]
        .into_iter()
        .filter(|(w, h)| {
            let p = plan(*w, *h, 30);
            p.frames == 30 && p.bytes <= MAX_BYTES
        })
        .count();
        assert_eq!(
            whole, 6,
            "every landscape and square shape measured keeps its whole clip"
        );

        assert_eq!(
            plan(64, 64, 10_000).frames,
            MAX_FRAMES,
            "a preview of tiny frames is capped by count"
        );
    }

    // ModlightLibraryTests.TheMotionBudgetNeverEnlargesASmallPreview
    #[test]
    fn the_motion_budget_never_enlarges_a_small_preview() {
        let small = plan(144, 256, 15);
        assert!(
            small.width == 144 && small.height == 256,
            "a 144x256 preview is not enlarged"
        );
        assert_eq!(small.frames, 15, "and keeps every frame it has");
        assert_eq!(mib(small.bytes), "2.11", "at 2.11 MiB");
    }

    // ModlightLibraryTests.TheMotionBudgetSurvivesNonsense
    #[test]
    fn the_motion_budget_survives_nonsense() {
        assert_eq!(
            plan(0, 480, 30).frames,
            0,
            "a zero-width canvas plans nothing"
        );
        assert_eq!(
            plan(480, 0, 30).frames,
            0,
            "a zero-height canvas plans nothing"
        );
        assert_eq!(
            plan(480, 480, 0).frames,
            0,
            "an artefact with no frames plans nothing"
        );
        assert_eq!(
            plan(480, 480, 1).frames,
            1,
            "a one-frame artefact is never asked for a second frame"
        );
    }

    #[test]
    fn the_budget_evicts_the_oldest_unpinned() {
        let mut budget = ThumbnailBudget::new(100);
        assert!(budget.add("a", 40).is_empty());
        assert!(budget.add("b", 40).is_empty());
        budget.pin(&["a"]);
        let released = budget.add("c", 40);
        assert_eq!(released, vec!["b"], "a is pinned, so b goes");
        assert_eq!(budget.bytes(), 80);
        budget.touch(&"a");
        budget.pin(&[]);
        let released = budget.add("d", 40);
        assert_eq!(
            released,
            vec!["c"],
            "a was touched, so the least recent is c"
        );
        budget.forget(&"a");
        assert_eq!(budget.count(), 1);
        budget.clear();
        assert_eq!(budget.bytes(), 0);
    }
}

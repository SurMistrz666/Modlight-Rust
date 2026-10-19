//! The order thumbnails are fetched in (`ThumbnailQueue.cs`, `VisibleBand.cs`).
//!
//! A fixed number of slots and a queue that is REPLACED, not appended to, every time
//! the visible range changes: what was waiting and is no longer wanted is dropped, and
//! the tiles on screen are served before the ones scrolled past. A semaphore would
//! serve them in arrival order, which is what made a fast scroll slow upstream.
//!
//! This version is synchronous: [`ThumbnailQueue::want`] and
//! [`ThumbnailQueue::finished`] return the items the caller should start fetching now.

use std::collections::HashSet;
use std::hash::Hash;

pub struct ThumbnailQueue<T: Eq + Hash + Clone> {
    slots: usize,
    queue: Vec<T>,
    queued: HashSet<T>,
    running: HashSet<T>,
    head: usize,
    issued: usize,
    abandoned: usize,
}

impl<T: Eq + Hash + Clone> ThumbnailQueue<T> {
    pub fn new(slots: usize) -> Self {
        ThumbnailQueue {
            slots: slots.max(1),
            queue: Vec::new(),
            queued: HashSet::new(),
            running: HashSet::new(),
            head: 0,
            issued: 0,
            abandoned: 0,
        }
    }

    /// Fetches started so far.
    pub fn issued(&self) -> usize {
        self.issued
    }

    /// Items that were waiting and were dropped before they started.
    pub fn abandoned(&self) -> usize {
        self.abandoned
    }

    pub fn running(&self) -> usize {
        self.running.len()
    }

    pub fn waiting(&self) -> usize {
        self.queued.len()
    }

    pub fn is_running(&self, item: &T) -> bool {
        self.running.contains(item)
    }

    /// Replace what is wanted, front first. Returns the fetches to start now.
    pub fn want(&mut self, ordered: &[T]) -> Vec<T> {
        let carried = ordered
            .iter()
            .filter(|item| self.queued.contains(*item))
            .count();
        self.abandoned += self.queued.len() - carried.min(self.queued.len());

        self.queue.clear();
        self.queued.clear();
        self.head = 0;

        for item in ordered {
            if !self.running.contains(item) && self.queued.insert(item.clone()) {
                self.queue.push(item.clone());
            }
        }
        self.pump()
    }

    /// Forget everything that is waiting (what is running finishes on its own).
    pub fn clear(&mut self) {
        self.abandoned += self.queued.len();
        self.queue.clear();
        self.queued.clear();
        self.head = 0;
    }

    /// Forget everything, running fetches included (a new listing replaced the grid).
    pub fn reset(&mut self) {
        self.clear();
        self.running.clear();
    }

    /// A fetch ended, however it ended. Returns the fetches to start now.
    pub fn finished(&mut self, item: &T) -> Vec<T> {
        self.running.remove(item);
        self.pump()
    }

    fn pump(&mut self) -> Vec<T> {
        let mut started = Vec::new();
        while self.running.len() < self.slots && self.head < self.queue.len() {
            let item = self.queue[self.head].clone();
            self.head += 1;
            if !self.queued.remove(&item) {
                continue;
            }
            self.running.insert(item.clone());
            self.issued += 1;
            started.push(item);
        }
        started
    }
}

/// Indices of a listing in the order their pictures should arrive: the visible run
/// first, then outwards a step at a time, below before above, up to `margin` away.
pub fn visible_band(
    count: usize,
    visible_first: usize,
    visible_last: usize,
    margin: usize,
) -> Vec<usize> {
    let mut order = Vec::new();
    if count == 0 {
        return order;
    }
    let first = visible_first.min(count - 1);
    let last = visible_last.clamp(first, count - 1);
    order.extend(first..=last);
    for step in 1..=margin {
        let below = last + step;
        if below < count {
            order.push(below);
        }
        if let Some(above) = first.checked_sub(step) {
            order.push(above);
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A queue whose fetches finish only when the test says so (the C# `Bench`).
    struct Bench<T: Eq + Hash + Clone + ToString> {
        queue: ThumbnailQueue<T>,
        in_flight: Vec<T>,
        started: Vec<T>,
        live: usize,
        peak: usize,
    }

    impl<T: Eq + Hash + Clone + ToString> Bench<T> {
        fn new(slots: usize) -> Self {
            Bench {
                queue: ThumbnailQueue::new(slots),
                in_flight: Vec::new(),
                started: Vec::new(),
                live: 0,
                peak: 0,
            }
        }

        fn start(&mut self, items: Vec<T>) {
            for item in items {
                self.started.push(item.clone());
                self.live += 1;
                self.peak = self.peak.max(self.live);
                self.in_flight.push(item);
            }
        }

        fn want(&mut self, items: &[T]) {
            let started = self.queue.want(items);
            self.start(started);
        }

        /// Let everything finish, including whatever finishing starts.
        fn drain(&mut self) {
            let mut i = 0;
            while i < self.in_flight.len() {
                let item = self.in_flight[i].clone();
                self.live -= 1;
                let started = self.queue.finished(&item);
                self.start(started);
                i += 1;
            }
            self.in_flight.clear();
        }

        fn order(&self) -> String {
            self.started
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(",")
        }
    }

    // ModlightLibraryTests.TheQueueServesTheFrontFirst
    #[test]
    fn the_queue_serves_the_front_first() {
        let mut bench = Bench::<&str>::new(2);
        bench.want(&["a", "b", "c", "d"]);
        assert_eq!(
            bench.order(),
            "a,b",
            "only as many start as there are slots"
        );
        bench.want(&["d", "c"]);
        bench.drain();
        assert_eq!(
            bench.order(),
            "a,b,d,c",
            "the reordered front is served next"
        );
    }

    // ModlightLibraryTests.TheQueueAbandonsWhatIsNoLongerWanted
    #[test]
    fn the_queue_abandons_what_is_no_longer_wanted() {
        let mut bench = Bench::<&str>::new(1);
        bench.want(&["a", "b", "c", "d", "e"]);
        assert!(
            bench.queue.issued() == 1 && bench.queue.waiting() == 4,
            "one in flight, four waiting"
        );
        bench.want(&["z"]);
        assert_eq!(
            bench.queue.abandoned(),
            4,
            "the four that were waiting are dropped, not fetched"
        );
        bench.drain();
        assert_eq!(
            bench.order(),
            "a,z",
            "only the one in flight and the one still wanted are fetched"
        );
    }

    // ModlightLibraryTests.TheQueueNeverExceedsItsSlots
    #[test]
    fn the_queue_never_exceeds_its_slots() {
        let mut bench = Bench::<i32>::new(4);
        for round in 0..20 {
            let want: Vec<i32> = (0..50).map(|i| round * 7 + i).collect();
            bench.want(&want);
            bench.drain();
        }
        assert!(
            bench.peak <= 4,
            "never more than 4 fetches in flight (peak {})",
            bench.peak
        );
    }

    // ModlightLibraryTests.TheBandIsOrderedOutwards
    #[test]
    fn the_band_is_ordered_outwards() {
        let order: Vec<String> = visible_band(100, 10, 12, 3)
            .iter()
            .map(|i| i.to_string())
            .collect();
        assert_eq!(
            order.join(","),
            "10,11,12,13,9,14,8,15,7",
            "visible first, then outwards"
        );
        let order: Vec<String> = visible_band(5, 0, 1, 10)
            .iter()
            .map(|i| i.to_string())
            .collect();
        assert_eq!(
            order.join(","),
            "0,1,2,3,4",
            "the band stops at the ends of the listing"
        );
    }

    // ModlightLibraryTests.MeasureAFastScroll: a flick through the listing in virtual
    // time, the shipped policy against the old first-come-first-served one.
    const LATENCY_MS: i64 = 238;
    const SLOTS: usize = 4;
    const MARGIN: usize = 40;
    const VISIBLE: usize = 24;

    type Event = Box<dyn FnOnce(&mut Sim)>;

    struct Clock {
        events: Vec<(i64, i64, Event)>,
        seq: i64,
        now: i64,
    }

    struct Sim {
        clock_events: Vec<(i64, Event)>,
        now: i64,
        arrived: Vec<bool>,
        arrived_at: Vec<i64>,
        pending: Vec<bool>,
        waiting: VecDeque<usize>,
        running: usize,
        issued: usize,
        queue: Option<ThumbnailQueue<usize>>,
    }

    impl Sim {
        fn at(&mut self, delay: i64, run: Event) {
            self.clock_events.push((self.now + delay, run));
        }

        fn finish(&mut self, card: usize) {
            self.arrived[card] = true;
            self.arrived_at[card] = self.now;
        }

        fn start_legacy(&mut self, card: usize) {
            self.running += 1;
            self.at(
                LATENCY_MS,
                Box::new(move |sim| {
                    sim.finish(card);
                    sim.running -= 1;
                    sim.pump_legacy();
                }),
            );
        }

        fn pump_legacy(&mut self) {
            while self.running < SLOTS {
                let Some(card) = self.waiting.pop_front() else {
                    break;
                };
                self.start_legacy(card);
            }
        }

        fn start_new(&mut self, cards: Vec<usize>) {
            for card in cards {
                self.at(
                    LATENCY_MS,
                    Box::new(move |sim| {
                        sim.finish(card);
                        let next = sim.queue.as_mut().unwrap().finished(&card);
                        sim.start_new(next);
                    }),
                );
            }
        }

        fn scroll(&mut self, first: usize, last: usize, legacy: bool) {
            let cards = self.arrived.len();
            if legacy {
                let band_first = first.saturating_sub(MARGIN);
                let band_last = (cards - 1).min(last + MARGIN);
                for card in band_first..=band_last {
                    if !self.arrived[card] && !self.pending[card] {
                        self.pending[card] = true;
                        self.issued += 1;
                        self.waiting.push_back(card);
                    }
                }
                self.pump_legacy();
                return;
            }
            let want: Vec<usize> = visible_band(cards, first, last, MARGIN)
                .into_iter()
                .filter(|card| !self.arrived[*card])
                .collect();
            let started = self.queue.as_mut().unwrap().want(&want);
            self.start_new(started);
        }
    }

    /// Returns (requests, first tile ms, full screen ms) after the scroll comes to rest.
    fn simulate(cards: usize, legacy: bool) -> (usize, i64, i64) {
        let mut sim = Sim {
            clock_events: Vec::new(),
            now: 0,
            arrived: vec![false; cards],
            arrived_at: vec![0; cards],
            pending: vec![false; cards],
            waiting: VecDeque::new(),
            running: 0,
            issued: 0,
            queue: (!legacy).then(|| ThumbnailQueue::new(SLOTS)),
        };
        let mut clock = Clock {
            events: Vec::new(),
            seq: 0,
            now: 0,
        };

        const EVENTS: usize = 50;
        let travel = 1.max((cards - VISIBLE).div_ceil(EVENTS));
        let mut at = 0usize;
        for step in 0..EVENTS {
            let top = at;
            clock.events.push((
                step as i64 * 16,
                clock.seq,
                Box::new(move |sim: &mut Sim| sim.scroll(top, top + VISIBLE - 1, legacy)),
            ));
            clock.seq += 1;
            at = (cards - VISIBLE).min(at + travel);
        }
        let rest_top = at;
        let rest_at = (EVENTS as i64 - 1) * 16;
        clock.events.push((
            rest_at,
            clock.seq,
            Box::new(move |sim: &mut Sim| sim.scroll(rest_top, rest_top + VISIBLE - 1, legacy)),
        ));
        clock.seq += 1;

        // Run: events in time order, ties in booking order.
        loop {
            for (time, run) in sim.clock_events.drain(..) {
                clock.events.push((time, clock.seq, run));
                clock.seq += 1;
            }
            if clock.events.is_empty() {
                break;
            }
            let best = (0..clock.events.len())
                .min_by_key(|&i| (clock.events[i].0, clock.events[i].1))
                .unwrap();
            let (time, _, run) = clock.events.remove(best);
            clock.now = time;
            sim.now = time;
            run(&mut sim);
        }

        let issued = if legacy {
            sim.issued
        } else {
            sim.queue.as_ref().unwrap().issued()
        };
        let mut first_ms = -1;
        let mut settled = -1;
        for card in rest_top..(rest_top + VISIBLE).min(cards) {
            if !sim.arrived[card] {
                return (issued, first_ms, -1);
            }
            let waited = sim.arrived_at[card] - rest_at;
            if first_ms < 0 || waited < first_ms {
                first_ms = waited;
            }
            settled = settled.max(waited);
        }
        (issued, first_ms, settled)
    }

    #[test]
    fn a_freshly_scrolled_to_screen_fills_faster_than_it_used_to() {
        for (what, cards) in [
            ("the endless grid, 1,200 cards loaded", 1200),
            ("one page, 120 cards", 120),
        ] {
            let old = simulate(cards, true);
            let new = simulate(cards, false);
            println!("{what}: before {old:?}, after {new:?} (requests, first ms, full screen ms)");
            assert!(
                new.2 >= 0 && new.2 < old.2,
                "faster after a flick ({what}): {new:?} vs {old:?}"
            );
        }
    }
}

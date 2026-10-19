//! `ShareX.UploadersLib.ProgressManager`: bytes moved, and a speed smoothed over the last
//! ten quarter-second samples, which is what the transfer rows print.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How often a new speed sample is taken (`smoothTime`).
const SMOOTH_TIME: Duration = Duration::from_millis(250);

/// How many samples the speed is the average of (`FixedSizedQueue<double>(10)`).
const SAMPLES: usize = 10;

#[derive(Clone, Debug)]
pub struct ProgressManager {
    position: i64,
    length: i64,
    speed: f64,
    started: Instant,
    finished: Option<Instant>,
    sample_started: Instant,
    speed_test: i64,
    samples: VecDeque<f64>,
}

impl ProgressManager {
    pub fn new(length: i64, position: i64) -> ProgressManager {
        ProgressManager::starting_at(length, position, Instant::now())
    }

    /// A manager whose clock starts at `now` (tests drive the clock by hand).
    pub fn starting_at(length: i64, position: i64, now: Instant) -> ProgressManager {
        ProgressManager {
            position,
            length,
            speed: 0.0,
            started: now,
            finished: None,
            sample_started: now,
            speed_test: 0,
            samples: VecDeque::with_capacity(SAMPLES),
        }
    }

    pub fn position(&self) -> i64 {
        self.position
    }

    pub fn length(&self) -> i64 {
        self.length
    }

    /// Bytes per second, smoothed.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// `Position / Length * 100`; NaN-free (a zero length reads as 0).
    pub fn percentage(&self) -> f64 {
        if self.length <= 0 {
            0.0
        } else {
            self.position as f64 / self.length as f64 * 100.0
        }
    }

    /// How long the transfer has run; stops counting when it reached the end.
    pub fn elapsed(&self) -> Duration {
        self.finished
            .unwrap_or_else(Instant::now)
            .saturating_duration_since(self.started)
    }

    /// Time left at the current speed; zero while there is no speed to divide by.
    pub fn remaining(&self) -> Duration {
        if self.speed > 0.0 {
            Duration::from_secs_f64(((self.length - self.position).max(0) as f64) / self.speed)
        } else {
            Duration::ZERO
        }
    }

    /// Count `bytes_read` more. Returns whether the caller should report progress now:
    /// at the end, and whenever a new speed sample was taken.
    pub fn update_progress(&mut self, bytes_read: i64) -> bool {
        self.update_progress_at(bytes_read, Instant::now())
    }

    pub fn update_progress_at(&mut self, bytes_read: i64, now: Instant) -> bool {
        self.position += bytes_read;
        self.speed_test += bytes_read;

        if self.position >= self.length {
            self.position = self.length;
            self.finished.get_or_insert(now);
            return true;
        }

        let since = now.saturating_duration_since(self.sample_started);
        if since > SMOOTH_TIME {
            if self.samples.len() == SAMPLES {
                self.samples.pop_front();
            }
            self.samples
                .push_back(self.speed_test as f64 / since.as_secs_f64());
            self.speed = self.samples.iter().sum::<f64>() / self.samples.len() as f64;
            self.speed_test = 0;
            self.sample_started = now;
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_is_the_average_of_the_samples() {
        let start = Instant::now();
        let mut progress = ProgressManager::starting_at(10_000_000, 0, start);
        assert!(!progress.update_progress_at(1000, start + Duration::from_millis(100)));
        // 300 ms in: one sample of 2000 bytes over 300 ms.
        assert!(progress.update_progress_at(1000, start + Duration::from_millis(300)));
        assert!((progress.speed() - 2000.0 / 0.3).abs() < 0.01);
        // A second sample at a different rate: the speed is the mean of the two.
        assert!(progress.update_progress_at(4000, start + Duration::from_millis(800)));
        let expected = (2000.0 / 0.3 + 4000.0 / 0.5) / 2.0;
        assert!((progress.speed() - expected).abs() < 0.01);
        assert!(progress.remaining() > Duration::ZERO);
    }

    #[test]
    fn the_end_is_reported_and_clamped() {
        let start = Instant::now();
        let mut progress = ProgressManager::starting_at(100, 90, start);
        assert!((progress.percentage() - 90.0).abs() < f64::EPSILON);
        assert!(progress.update_progress_at(50, start + Duration::from_millis(10)));
        assert_eq!(progress.position(), 100);
        assert_eq!(progress.percentage(), 100.0);
    }

    #[test]
    fn an_empty_transfer_has_no_nan() {
        let progress = ProgressManager::new(0, 0);
        assert_eq!(progress.percentage(), 0.0);
        assert_eq!(progress.remaining(), Duration::ZERO);
    }

    #[test]
    fn samples_roll_after_ten() {
        let start = Instant::now();
        let mut progress = ProgressManager::starting_at(i64::MAX, 0, start);
        for i in 1..=12u64 {
            progress.update_progress_at(1000, start + Duration::from_millis(300 * i));
        }
        assert_eq!(progress.samples.len(), SAMPLES);
    }
}

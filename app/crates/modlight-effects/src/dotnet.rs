//! .NET pieces whose exact output the effects depend on: `new Random(seed)` (the Knuth
//! subtractive generator .NET keeps for seeded instances) and `Random.Shared` seeds.

/// `System.Random` constructed with a seed (`Net5CompatSeedImpl`).
#[derive(Clone, Debug)]
pub struct DotNetRandom {
    seed_array: [i32; 56],
    inext: usize,
    inextp: usize,
}

const MBIG: i32 = i32::MAX;
const MSEED: i32 = 161803398;

impl DotNetRandom {
    pub fn new(seed: i32) -> DotNetRandom {
        let mut sa = [0i32; 56];
        let subtraction = if seed == i32::MIN {
            i32::MAX
        } else {
            seed.abs()
        };
        let mut mj = MSEED - subtraction;
        sa[55] = mj;
        let mut mk = 1i32;
        let mut ii = 0usize;
        for _ in 1..55 {
            ii += 21;
            if ii >= 55 {
                ii -= 55;
            }
            sa[ii] = mk;
            mk = mj.wrapping_sub(mk);
            if mk < 0 {
                mk = mk.wrapping_add(MBIG);
            }
            mj = sa[ii];
        }
        for _ in 1..5 {
            for i in 1..56 {
                let mut n = i + 30;
                if n >= 55 {
                    n -= 55;
                }
                sa[i] = sa[i].wrapping_sub(sa[1 + n]);
                if sa[i] < 0 {
                    sa[i] = sa[i].wrapping_add(MBIG);
                }
            }
        }
        DotNetRandom {
            seed_array: sa,
            inext: 0,
            inextp: 21,
        }
    }

    fn internal_sample(&mut self) -> i32 {
        let mut a = self.inext + 1;
        if a >= 56 {
            a = 1;
        }
        let mut b = self.inextp + 1;
        if b >= 56 {
            b = 1;
        }
        let mut ret = self.seed_array[a].wrapping_sub(self.seed_array[b]);
        if ret == MBIG {
            ret -= 1;
        }
        if ret < 0 {
            ret = ret.wrapping_add(MBIG);
        }
        self.seed_array[a] = ret;
        self.inext = a;
        self.inextp = b;
        ret
    }

    fn sample(&mut self) -> f64 {
        self.internal_sample() as f64 * (1.0 / MBIG as f64)
    }

    fn large_range_sample(&mut self) -> f64 {
        let mut result = self.internal_sample();
        let negative = self.internal_sample() % 2 == 0;
        if negative {
            result = -result;
        }
        let mut d = result as f64;
        d += (i32::MAX - 1) as f64;
        d /= 2.0 * i32::MAX as u32 as f64 - 1.0;
        d
    }

    /// `Next()`.
    pub fn next_int(&mut self) -> i32 {
        self.internal_sample()
    }

    /// `Next(maxValue)`.
    pub fn next_max(&mut self, max: i32) -> i32 {
        (self.sample() * max as f64) as i32
    }

    /// `Next(minValue, maxValue)`.
    pub fn next_range(&mut self, min: i32, max: i32) -> i32 {
        let range = max as i64 - min as i64;
        if range <= i32::MAX as i64 {
            (self.sample() * range as f64) as i32 + min
        } else {
            ((self.large_range_sample() * range as f64) as i64 + min as i64) as i32
        }
    }

    /// `NextDouble()`.
    pub fn next_double(&mut self) -> f64 {
        self.sample()
    }

    /// `NextSingle()`.
    pub fn next_single(&mut self) -> f32 {
        self.sample() as f32
    }
}

/// `Random.Shared.Next(min, max)`: a fresh unpredictable value (the effects that use it
/// differ on every run, upstream as here).
pub fn shared_next(min: i32, max: i32) -> i32 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut h = RandomState::new().build_hasher();
    h.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0),
    );
    let span = (max as i64 - min as i64).max(1) as u64;
    (min as i64 + (h.finish() % span) as i64) as i32
}

/// An unseeded `new Random()`.
pub fn shared_random() -> DotNetRandom {
    DotNetRandom::new(shared_next(0, i32::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_dotnet_seeded_sequences() {
        // new Random(42): Next() = 1434747710, 302596119; Next(0, 100) after that.
        let mut r = DotNetRandom::new(42);
        assert_eq!(r.next_int(), 1434747710);
        assert_eq!(r.next_int(), 302596119);
        let mut r = DotNetRandom::new(0);
        assert_eq!(r.next_int(), 1559595546);
    }
}

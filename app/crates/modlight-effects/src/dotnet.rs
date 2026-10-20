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

/// `Array.Sort(array, comparison)`: .NET's introspective sort (`ArraySortHelper<T>`), which
/// is not stable, so equal keys end up where .NET puts them.
pub fn sort_by<T: Copy>(keys: &mut [T], cmp: &impl Fn(&T, &T) -> std::cmp::Ordering) {
    if keys.len() > 1 {
        let depth = 2 * ((keys.len() as u32).ilog2() as i32 + 1);
        intro_sort(keys, depth, cmp);
    }
}

fn swap_if_greater<T: Copy>(
    keys: &mut [T],
    cmp: &impl Fn(&T, &T) -> std::cmp::Ordering,
    i: usize,
    j: usize,
) {
    if cmp(&keys[i], &keys[j]) == std::cmp::Ordering::Greater {
        keys.swap(i, j);
    }
}

fn intro_sort<T: Copy>(
    keys: &mut [T],
    mut depth: i32,
    cmp: &impl Fn(&T, &T) -> std::cmp::Ordering,
) {
    use std::cmp::Ordering::Less;
    let mut size = keys.len();
    while size > 1 {
        if size <= 16 {
            if size == 2 {
                swap_if_greater(keys, cmp, 0, 1);
                return;
            }
            if size == 3 {
                swap_if_greater(keys, cmp, 0, 1);
                swap_if_greater(keys, cmp, 0, 2);
                swap_if_greater(keys, cmp, 1, 2);
                return;
            }
            // Insertion sort.
            let k = &mut keys[..size];
            for i in 0..k.len() - 1 {
                let t = k[i + 1];
                let mut j = i as isize;
                while j >= 0 && cmp(&t, &k[j as usize]) == Less {
                    k[(j + 1) as usize] = k[j as usize];
                    j -= 1;
                }
                k[(j + 1) as usize] = t;
            }
            return;
        }
        if depth == 0 {
            heap_sort(&mut keys[..size], cmp);
            return;
        }
        depth -= 1;
        // Median of three, then partition.
        let k = &mut keys[..size];
        let hi = k.len() - 1;
        let middle = hi >> 1;
        swap_if_greater(k, cmp, 0, middle);
        swap_if_greater(k, cmp, 0, hi);
        swap_if_greater(k, cmp, middle, hi);
        let pivot = k[middle];
        k.swap(middle, hi - 1);
        let (mut left, mut right) = (0usize, hi - 1);
        while left < right {
            loop {
                left += 1;
                if cmp(&k[left], &pivot) != Less {
                    break;
                }
            }
            loop {
                right -= 1;
                if cmp(&pivot, &k[right]) != Less {
                    break;
                }
            }
            if left >= right {
                break;
            }
            k.swap(left, right);
        }
        if left != hi - 1 {
            k.swap(left, hi - 1);
        }
        let p = left;
        intro_sort(&mut keys[p + 1..size], depth, cmp);
        size = p;
    }
}

fn heap_sort<T: Copy>(keys: &mut [T], cmp: &impl Fn(&T, &T) -> std::cmp::Ordering) {
    let n = keys.len();
    let mut i = n >> 1;
    while i >= 1 {
        down_heap(keys, i, n, cmp);
        i -= 1;
    }
    let mut i = n;
    while i > 1 {
        keys.swap(0, i - 1);
        down_heap(keys, 1, i - 1, cmp);
        i -= 1;
    }
}

fn down_heap<T: Copy>(
    keys: &mut [T],
    mut i: usize,
    n: usize,
    cmp: &impl Fn(&T, &T) -> std::cmp::Ordering,
) {
    use std::cmp::Ordering::Less;
    let d = keys[i - 1];
    while i <= n >> 1 {
        let mut child = 2 * i;
        if child < n && cmp(&keys[child - 1], &keys[child]) == Less {
            child += 1;
        }
        if cmp(&d, &keys[child - 1]) != Less {
            break;
        }
        keys[i - 1] = keys[child - 1];
        i = child;
    }
    keys[i - 1] = d;
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

    #[test]
    fn sort_by_orders_like_a_sort() {
        let mut v: Vec<i32> = (0..500).map(|i| (i * 7919) % 211).collect();
        let mut w = v.clone();
        super::sort_by(&mut v, &|a: &i32, b: &i32| a.cmp(b));
        w.sort();
        assert_eq!(v, w);
    }
}

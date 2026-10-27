//! Fractional-index order keys (spec todo-sync §5.6, plan §3.5).
//!
//! The local queue has no sort key (order = array position), so every synced todo carries a string key
//! and the cloud order is `(key, hlc, id)`. A key is a non-empty base-62 string that does not end in
//! `'0'` (the digit alphabet is in ASCII order, so plain string comparison is the key order), and a new
//! key can always be minted strictly between two existing ones — dragging a todo only rewrites the
//! moved todo's key.

/// Digit alphabet in ascending ASCII order: `0-9`, `A-Z`, `a-z`.
const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const BASE: usize = 62;

/// Longer keys trigger a one-off renumber of the whole project by whoever noticed (spec §5.6).
pub const MAX_LEN: usize = 32;

fn digit(c: u8) -> Option<usize> {
    DIGITS.iter().position(|&d| d == c)
}

/// Whether `key` is a well-formed order key.
pub fn is_valid(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|c| digit(c).is_some()) && !key.ends_with('0')
}

fn first_digit(s: &str) -> usize {
    s.bytes().next().and_then(digit).unwrap_or(0)
}

/// Midpoint strictly between `a` (empty = the lowest possible key) and `b` (`None` = no upper bound).
/// Requires `a < b`, and neither may end in `'0'`.
fn midpoint(a: &str, b: Option<&str>) -> String {
    if let Some(b) = b {
        // Shared prefix (a is implicitly padded with '0's): keep it and recurse on the remainders.
        let (ab, bb) = (a.as_bytes(), b.as_bytes());
        let mut n = 0;
        while n < bb.len() && ab.get(n).copied().unwrap_or(b'0') == bb[n] {
            n += 1;
        }
        if n > 0 {
            let a_rest = if n <= a.len() { &a[n..] } else { "" };
            return format!("{}{}", &b[..n], midpoint(a_rest, Some(&b[n..])));
        }
    }
    let da = first_digit(a);
    let db = b.map_or(BASE, first_digit);
    if db - da > 1 {
        return (DIGITS[(da + db).div_ceil(2)] as char).to_string();
    }
    // The first digits are adjacent: either b is longer (its first digit alone is already between), or
    // keep a's first digit and find room in the remainder.
    match b {
        Some(b) if b.len() > 1 => b[..1].to_string(),
        _ => {
            let rest = if a.len() > 1 { &a[1..] } else { "" };
            format!("{}{}", DIGITS[da] as char, midpoint(rest, None))
        }
    }
}

/// A key strictly after `a` (`None` = the queue is empty).
pub fn after(a: Option<&str>) -> String {
    match a.filter(|a| !a.is_empty()) {
        None => midpoint("", None),
        Some(a) => {
            let d = first_digit(a);
            if d < BASE - 1 {
                (DIGITS[d + 1] as char).to_string()
            } else {
                format!("z{}", after(Some(&a[1..])))
            }
        }
    }
}

/// A key strictly before `b`.
pub fn before(b: &str) -> String {
    if b.is_empty() {
        return after(None);
    }
    let d = first_digit(b);
    match d {
        0 => format!("0{}", before(&b[1..])),
        1 => "0V".to_string(),
        _ => (DIGITS[d - 1] as char).to_string(),
    }
}

/// A key strictly between `a` and `b` (`None` = open end). If the pair is out of order (corrupt or
/// racing data) the result is still a valid key after `a`; the `(key, hlc, id)` tie-break keeps the
/// overall order total either way.
pub fn between(a: Option<&str>, b: Option<&str>) -> String {
    match (a.filter(|a| !a.is_empty()), b.filter(|b| !b.is_empty())) {
        (None, None) => after(None),
        (Some(a), None) => after(Some(a)),
        (None, Some(b)) => before(b),
        (Some(a), Some(b)) => {
            if a < b {
                midpoint(a, Some(b))
            } else {
                after(Some(a))
            }
        }
    }
}

/// `n` evenly spaced fresh keys in ascending order (initial assignment and the long-key reset).
pub fn renumber(n: usize) -> Vec<String> {
    let mut width = 1;
    let mut capacity = BASE;
    while capacity < n {
        width += 1;
        capacity = capacity.saturating_mul(BASE);
    }
    (0..n)
        .map(|i| {
            let mut digits = vec![b'0'; width];
            let mut v = i;
            for slot in digits.iter_mut().rev() {
                *slot = DIGITS[v % BASE];
                v /= BASE;
            }
            // A constant non-zero last digit keeps every key valid and leaves room after each.
            digits.push(b'V');
            String::from_utf8(digits).unwrap_or_default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// xorshift: tiny deterministic generator so failures reproduce from the printed seed.
    fn rng(seed: u64) -> impl FnMut() -> u64 {
        let mut s = seed | 1;
        move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        }
    }

    #[test]
    fn first_and_adjacent_keys() {
        assert_eq!(after(None), "V");
        assert!(after(Some("V")).as_str() > "V");
        assert!(before("V").as_str() < "V");
        assert_eq!(between(Some("U"), Some("V")), "UV");
        assert_eq!(between(None, Some("1")), "0V");
        assert!(
            is_valid("0V")
                && is_valid("zzV")
                && !is_valid("a0")
                && !is_valid("")
                && !is_valid("a-")
        );
    }

    #[test]
    fn appending_and_prepending_stay_ordered_and_short() {
        let mut keys = vec![after(None)];
        for _ in 0..500 {
            let next = after(keys.last().map(String::as_str));
            assert!(is_valid(&next));
            assert!(next > *keys.last().unwrap());
            keys.push(next);
        }
        assert!(keys.last().unwrap().len() <= 20, "{}", keys.last().unwrap());

        let mut keys = vec![after(None)];
        for _ in 0..500 {
            let prev = before(&keys[0]);
            assert!(is_valid(&prev), "{prev}");
            assert!(prev < keys[0]);
            keys.insert(0, prev);
        }
    }

    #[test]
    fn random_insertions_always_land_strictly_between_neighbours() {
        for seed in 1..=20u64 {
            let mut next = rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let mut keys: Vec<String> = Vec::new();
            for step in 0..400 {
                let pos = (next() as usize) % (keys.len() + 1);
                let lo = pos.checked_sub(1).map(|i| keys[i].as_str());
                let hi = keys.get(pos).map(String::as_str);
                let key = between(lo, hi);
                assert!(is_valid(&key), "seed {seed} step {step}: invalid {key}");
                if let Some(lo) = lo {
                    assert!(key.as_str() > lo, "seed {seed} step {step}: {key} !> {lo}");
                }
                if let Some(hi) = hi {
                    assert!(key.as_str() < hi, "seed {seed} step {step}: {key} !< {hi}");
                }
                keys.insert(pos, key);
            }
            let mut sorted = keys.clone();
            sorted.sort();
            assert_eq!(keys, sorted, "seed {seed}");
        }
    }

    #[test]
    fn repeated_bisection_grows_slowly_enough_to_need_a_reset_only_rarely() {
        let (lo, mut hi) = ("V".to_string(), "W".to_string());
        for _ in 0..60 {
            hi = between(Some(&lo), Some(&hi));
            assert!(hi > lo);
        }
        assert!(hi.len() <= 70, "{} chars", hi.len());
    }

    #[test]
    fn out_of_order_input_still_yields_a_valid_key() {
        let key = between(Some("b"), Some("a"));
        assert!(is_valid(&key) && key.as_str() > "b");
    }

    #[test]
    fn renumber_is_ascending_valid_and_short() {
        for n in [0usize, 1, 2, 61, 62, 63, 3844, 3845, 5000] {
            let keys = renumber(n);
            assert_eq!(keys.len(), n);
            assert!(keys.iter().all(|k| is_valid(k) && k.len() <= 5));
            assert!(keys.windows(2).all(|w| w[0] < w[1]), "n={n}");
        }
        // Room remains between and after renumbered keys.
        let keys = renumber(3);
        assert!(between(Some(&keys[0]), Some(&keys[1])) < keys[1]);
        assert!(after(Some(&keys[2])) > keys[2]);
    }
}

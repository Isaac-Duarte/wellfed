//! The UTC timestamps evidence and manifests carry. Each keeps the exact
//! text the former Python tools wrote, which frozen reports and manifests
//! still hold: microseconds, truncated, and an explicit `+00:00`.

use jiff::Timestamp;

/// `at` with exactly six fractional digits: `2026-10-17T21:17:41.123456+00:00`.
pub fn micros(at: Timestamp) -> String {
    at.strftime("%Y-%m-%dT%H:%M:%S%.6f+00:00").to_string()
}

/// `at` as Python's `datetime.isoformat()` gives it: as [`micros`], but with
/// no fraction at all when its microseconds are zero.
pub fn isoformat(at: Timestamp) -> String {
    if at.subsec_microsecond() == 0 {
        at.strftime("%Y-%m-%dT%H:%M:%S+00:00").to_string()
    } else {
        micros(at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(nanos: i32) -> Timestamp {
        Timestamp::new(1_792_271_861, nanos).unwrap()
    }

    #[test]
    fn micros_always_carries_six_truncated_digits() {
        assert_eq!(micros(at(0)), "2026-10-17T21:17:41.000000+00:00");
        assert_eq!(micros(at(123_456_789)), "2026-10-17T21:17:41.123456+00:00");
        assert_eq!(micros(at(999_999_999)), "2026-10-17T21:17:41.999999+00:00");
    }

    #[test]
    fn isoformat_drops_the_fraction_only_on_a_whole_second() {
        assert_eq!(isoformat(at(0)), "2026-10-17T21:17:41+00:00");
        assert_eq!(isoformat(at(999)), "2026-10-17T21:17:41+00:00");
        assert_eq!(isoformat(at(1_000)), "2026-10-17T21:17:41.000001+00:00");
        assert_eq!(
            isoformat(at(123_456_789)),
            "2026-10-17T21:17:41.123456+00:00"
        );
        assert_eq!(
            isoformat(at(500_000_000)),
            "2026-10-17T21:17:41.500000+00:00"
        );
    }
}

use std::net::{Ipv4Addr, Ipv6Addr};

use super::*;

const T0: i64 = 1_790_000_000_000;

fn at(ms: i64) -> Timestamp {
    Timestamp::from_millisecond(T0 + ms).unwrap()
}

fn v4(last: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
}

/// `name` from `from` fails `n` times at `now`, every one taken.
fn fail(limits: &Limits, name: &str, from: IpAddr, now: Timestamp, n: usize) {
    for attempt in 0..n {
        let taken = limits.attempt(name, from, now);
        assert!(taken.is_ok(), "{name} attempt {attempt}");
    }
}

#[test]
fn a_name_takes_five_failures_a_minute_from_an_address_then_one_every_twelve_seconds() {
    let limits = Limits::default();

    fail(&limits, "ada", v4(1), at(0), 5);
    let refused = limits.attempt("Ada ", v4(1), at(0)).err();
    assert_eq!(
        refused,
        Some(Limited {
            retry_after: Duration::from_secs(12)
        }),
        "another spelling is the same name"
    );

    assert!(limits.attempt("ada", v4(1), at(11_999)).is_err());
    assert!(limits.attempt("ada", v4(1), at(12_000)).is_ok());
    assert!(limits.attempt("ada", v4(1), at(12_000)).is_err());
    assert!(
        limits.attempt("bob", v4(1), at(12_000)).is_ok(),
        "other names carry on"
    );
}

/// Someone guessing at a name from their address locks out that address
/// alone: the owner logging in from another is not limited by it.
#[test]
fn one_address_flooding_a_name_leaves_another_address_free() {
    let limits = Limits::default();
    fail(&limits, "ada", v4(1), at(0), 5);
    for _ in 0..100 {
        assert!(limits.attempt("ada", v4(1), at(0)).is_err());
    }

    let attempt = limits.attempt("ada", v4(2), at(0));
    assert!(attempt.is_ok(), "{:?}", attempt.err());
}

/// Guessing spread over many addresses meets the name's ceiling: 30 a
/// minute from every address together, then one every two seconds.
#[test]
fn a_name_takes_thirty_failures_a_minute_across_addresses() {
    let limits = Limits::default();
    for n in 0..30 {
        fail(&limits, "ada", v4(n), at(0), 1);
    }

    let refused = limits.attempt("ada", v4(200), at(0)).err();
    assert_eq!(
        refused,
        Some(Limited {
            retry_after: Duration::from_secs(2)
        })
    );
    assert!(limits.attempt("ada", v4(201), at(1_999)).is_err());
    assert!(limits.attempt("ada", v4(201), at(2_000)).is_ok());
    assert!(limits.attempt("bob", v4(202), at(2_000)).is_ok());
}

#[test]
fn an_address_takes_twenty_failures_a_minute_across_names() {
    let limits = Limits::default();
    let from = v4(7);

    for n in 0..20 {
        assert!(limits.attempt(&format!("name {n}"), from, at(0)).is_ok());
    }
    let refused = limits.attempt("someone", from, at(0)).err();
    assert_eq!(
        refused,
        Some(Limited {
            retry_after: Duration::from_secs(3)
        })
    );
    assert!(limits.attempt("someone", v4(8), at(0)).is_ok());
    assert!(limits.attempt("someone else", from, at(3_000)).is_ok());
}

#[test]
fn a_refused_attempt_spends_nothing() {
    let limits = Limits::default();
    fail(&limits, "ada", v4(1), at(0), 5);

    for _ in 0..100 {
        assert!(limits.attempt("ada", v4(1), at(0)).is_err());
    }

    // The address's own bucket spent nothing on them: 15 shares are left.
    fail(&limits, "bob", v4(1), at(0), 5);
    assert!(limits.attempt("ada", v4(1), at(12_000)).is_ok());
}

#[test]
fn a_login_that_succeeds_gives_its_attempt_back() {
    let limits = Limits::default();

    for _ in 0..50 {
        let attempt = limits.attempt("ada", v4(1), at(0)).unwrap();
        limits.succeeded(attempt, at(0));
    }

    assert_eq!(limits.len(), 0, "full buckets are dropped");
    fail(&limits, "ada", v4(1), at(0), 5);
}

#[test]
fn an_ipv6_host_counts_by_its_64() {
    let limits = Limits::default();
    let host = |last: u16| IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 1, 0, 0, 0, last));

    for n in 0..20 {
        assert!(limits.attempt(&format!("name {n}"), host(n), at(0)).is_ok());
    }

    assert!(limits.attempt("other", host(999), at(0)).is_err());
    assert_eq!(Key::block(host(1)), Key::block(host(999)));
    let next_block = IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 2, 0, 0, 0, 1));
    assert!(limits.attempt("other", next_block, at(0)).is_ok());
    let mapped = IpAddr::V6(Ipv4Addr::new(192, 0, 2, 1).to_ipv6_mapped());
    assert_eq!(Key::block(mapped), Key::block(v4(1)));
}

#[test]
fn a_clock_stepped_back_limits_for_a_minute_at_most() {
    let limits = Limits::default();
    fail(&limits, "ada", v4(1), at(0), 5);

    let back = limits.attempt("ada", v4(1), at(-3_600_000)).err();

    assert_eq!(
        back,
        Some(Limited {
            retry_after: Duration::from_secs(12)
        })
    );
    // The wait it named holds: the bucket counts from the new time, not
    // from the old deadline an hour ahead.
    assert!(limits.attempt("ada", v4(1), at(-3_588_001)).is_err());
    assert!(limits.attempt("ada", v4(1), at(-3_588_000)).is_ok());
}

/// A fresh IPv6 /64 for each `n`, so every flood attempt needs three new
/// buckets.
fn block(n: u32) -> IpAddr {
    let [a, b, c, d] = n.to_be_bytes();
    let segment = |high: u8, low: u8| u16::from_be_bytes([high, low]);
    IpAddr::V6(Ipv6Addr::new(
        0x2001,
        0xdb8,
        segment(a, b),
        segment(c, d),
        0,
        0,
        0,
        1,
    ))
}

/// Between each of the target's attempts, 10,000 fresh names from fresh
/// /64s, each its own three new buckets. The map stays bounded, the
/// target's bucket is never pushed out, and it stays limited throughout:
/// the flood is refused instead, until buckets are full again.
#[test]
fn a_flood_of_new_keys_never_frees_the_target() {
    let limits = Limits::default();
    fail(&limits, "owner", v4(1), at(0), 5);

    let mut next = 0;
    for round in 0..5 {
        let now = at(round * 2_000);
        for _ in 0..10_000 {
            let _ = limits.attempt(&format!("flood {next}"), block(next), now);
            next += 1;
        }

        assert!(limits.len() <= ENTRIES, "{} buckets", limits.len());
        let target = limits.attempt("owner", v4(1), now);
        assert!(target.is_err(), "round {round}");
    }
}

/// Once the flood's buckets are full again they make room, and others log
/// in as before.
#[test]
fn a_full_map_takes_new_keys_once_its_buckets_refill() {
    let limits = Limits::default();
    for n in 0..10_000 {
        let _ = limits.attempt(&format!("flood {n}"), block(n), at(0));
    }

    let refused = limits.attempt("someone", v4(9), at(0)).err();
    assert_eq!(
        refused,
        Some(Limited {
            retry_after: Duration::from_secs(2)
        }),
        "the first flood bucket, a name's ceiling, is full again in 2 s"
    );

    assert!(limits.attempt("someone", v4(9), at(12_000)).is_ok());
    assert!(limits.len() <= ENTRIES);
}

/// A full map making room never drops the attempt's own bucket, even when
/// it is among the first full again: the attempt would set it straight
/// back and pass the bound.
#[test]
fn making_room_never_drops_the_attempts_own_bucket() {
    let limits = Limits::default();
    fail(&limits, "owner", v4(1), at(0), 1);
    for n in 0..3_332 {
        let _ = limits.attempt(&format!("flood {n}"), block(n), at(11_000));
    }
    // One bucket more: an old name from another of the flood's blocks.
    let _ = limits.attempt("flood 0", block(1), at(11_000));
    assert_eq!(limits.len(), ENTRIES);

    // The address's bucket is the attempt's own, and kept; the owner's two
    // name buckets, full again, make the room.
    assert!(limits.attempt("other", v4(1), at(12_000)).is_ok());
    assert_eq!(limits.len(), ENTRIES);
}

#[test]
fn a_long_name_is_kept_short() {
    let long = "x".repeat(70_000);

    let kept = Key::spelling(&long);

    assert_eq!(kept.chars().count(), NAME_CHARS);
}

/// Ada logs in from `from` at `now`.
fn login(limits: &Limits, from: IpAddr, now: Timestamp) {
    let attempt = limits.attempt("ada", from, now).unwrap();
    limits.succeeded(attempt, now);
}

/// Six other addresses hold the name's ceiling down, which stops a new
/// address. The one Ada logged in from still takes its own five a minute,
/// until a month has passed since that login.
#[test]
fn a_flood_from_six_addresses_leaves_a_known_address_free() {
    let limits = Limits::default();
    login(&limits, v4(1), at(0));

    for n in 10..16 {
        fail(&limits, "ada", v4(n), at(1_000), 5);
    }
    assert!(limits.attempt("ada", v4(20), at(1_000)).is_err());
    fail(&limits, "ada", v4(1), at(1_000), 5);
    assert!(
        limits.attempt("ada", v4(1), at(1_000)).is_err(),
        "its own bucket still limits it"
    );
    assert!(
        limits.attempt("bob", v4(1), at(1_000)).is_ok(),
        "known for Ada's name alone"
    );

    let month = KNOWN_FOR_MS;
    for n in 10..16 {
        fail(&limits, "ada", v4(n), at(month), 5);
    }
    assert!(limits.attempt("ada", v4(1), at(month)).is_err());
}

/// One name keeps at most its sixteen latest addresses, so logging in from
/// many pushes out only that name's oldest, and the map keeps no more
/// than its bound in all.
#[test]
fn known_addresses_are_bounded_per_name_and_in_all() {
    let limits = Limits::default();
    for n in 0..20 {
        login(&limits, v4(n), at(i64::from(n)));
    }
    assert_eq!(limits.known_len(), KNOWN_PER_NAME);
    for n in 30..36 {
        fail(&limits, "ada", v4(n), at(100), 5);
    }
    assert!(limits.attempt("ada", v4(0), at(100)).is_err(), "pushed out");
    assert!(limits.attempt("ada", v4(19), at(100)).is_ok());

    let limits = Limits::default();
    for n in 0..u32::try_from(KNOWN).unwrap() + 10 {
        let attempt = limits.attempt(&format!("user{n}"), v4(1), at(0)).unwrap();
        limits.succeeded(attempt, at(0));
    }
    assert_eq!(limits.known_len(), KNOWN);
}

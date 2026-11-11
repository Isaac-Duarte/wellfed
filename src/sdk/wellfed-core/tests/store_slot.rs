//! The dual-slot store under its torn-write rule, apart from any store's
//! policy: the shared header and trailer, what each state of a unit reads
//! as, the newest-valid-wins pick, and a power cut at every mutation and a
//! failure at every call of a write, for both extents and several unit
//! sizes. Each store's own power-cut test then checks its policy over the
//! same rule. Then the versions: a newer minor read and rewritten with the
//! bytes this build doesn't know, an older minor or a previous major
//! handed to the decoder, and a newer major refused.

use wellfed_core::store::slot::{
    CRC_AT, DualSlot, Extent, Found, Layout, Pick, Prepare, RECORD_BYTES, Version, crc32, get_u64,
    newest, put_u64, seal, target, unwritten,
};
use wellfed_testkit::journal_flash::{FakeJournalFlash, FlashIo, ORDERS, ProgramOrder};

/// Fields at 12..21: the generation and one payload byte.
const UNIT: Layout = Layout::new(*b"PLFTEST1", Version::new(3, 0), 24, Extent::Unit);
const RECORD: Layout = Layout {
    extent: Extent::Record,
    ..UNIT
};
/// Unit sizes: the record exactly, a tail shorter than a chunk, and a
/// tail of whole chunks. A sector's every call is covered once, by the
/// failure test; cutting at each of its mutations proves nothing more.
const SIZES: [usize; 3] = [RECORD_BYTES, 148, 256];
const SECTOR: usize = 4096;

fn units(f: &mut FakeJournalFlash, layout: &'static Layout) -> DualSlot<FlashIo> {
    // SAFETY: every test keeps its flash in place while the units are used.
    DualSlot::new(unsafe { f.io() }, layout)
}

/// A record of generation `generation`, its payload byte `value` at 20.
fn record(layout: &Layout, generation: u64, value: u8) -> [u8; RECORD_BYTES] {
    let mut p = layout.record();
    put_u64::<12, _>(&mut p, generation);
    p[20] = value;
    seal(&mut p);
    p
}

/// The generation and the payload; anything else in the fields is refused.
fn decode(_: Version, p: &[u8; RECORD_BYTES]) -> Option<(u64, u8)> {
    let generation = get_u64::<12, _>(p);
    (generation != 0 && p[21..CRC_AT].iter().all(|&b| b == 0)).then_some((generation, p[20]))
}

fn scan(f: &mut FakeJournalFlash, layout: &'static Layout, slot: u32) -> Found<(u64, u8)> {
    units(f, layout)
        .scan(slot, decode)
        .expect("the fake flash reads")
}

/// Store `p` in unit `slot` directly, as another writer or decay would.
fn place(f: &mut FakeJournalFlash, slot: usize, p: &[u8]) {
    f.bytes[slot][..p.len()].copy_from_slice(p);
}

#[test]
fn the_header_and_trailer_are_the_shared_layout() {
    let p = record(&UNIT, 0x0102_0304_0506_0708, 0x5a);

    assert_eq!(&p[..8], b"PLFTEST1");
    assert_eq!(&p[8..12], &[3, 0, 128, 0]);
    assert_eq!(&p[12..21], &[8, 7, 6, 5, 4, 3, 2, 1, 0x5a]);
    assert_eq!(p[120..124], crc32(&p[..120]).to_le_bytes());
    assert_eq!(&p[124..], b"CMIT");

    // A record of another length carries that length and its trailer at
    // its own end.
    let mut long: [u8; 584] = UNIT.record();
    seal(&mut long);
    assert_eq!(&long[10..12], &584u16.to_le_bytes());
    assert_eq!(long[576..580], crc32(&long[..576]).to_le_bytes());
    assert_eq!(&long[580..], b"CMIT");
}

#[test]
fn each_state_of_a_unit_reads_as_the_rule_names_it() {
    for layout in [&UNIT, &RECORD] {
        let valid = record(layout, 2, 9);
        let mut f = FakeJournalFlash::with_unit(256);
        assert_eq!(scan(&mut f, layout, 0), Found::Erased);

        place(&mut f, 0, &valid);
        assert_eq!(scan(&mut f, layout, 0), Found::Valid((2, 9)));

        // Programmed in order and cut short: no marker.
        for cut in [1, 5, 8, 9, 64, 124] {
            let mut f = FakeJournalFlash::with_unit(256);
            place(&mut f, 0, &valid[..cut]);
            assert_eq!(scan(&mut f, layout, 0), Found::Partial, "cut at {cut}");
        }

        // Cut inside a page program, which keeps no byte order: any subset
        // of the record's cleared bits, the marker still erased.
        for seed in [1, 2, 3] {
            for budget in [1, 20, 60, 110] {
                let mut f = FakeJournalFlash {
                    budget,
                    ..FakeJournalFlash::with_unit(256).ordered(ProgramOrder::Scattered(seed))
                };
                assert!(!units(&mut f, layout).write(0, &valid, Prepare::Erased));
                assert_eq!(
                    scan(&mut f, layout, 0),
                    Found::Partial,
                    "seed {seed} cut {budget}"
                );
            }
        }

        // No marker, and a bit the store's magic keeps set is clear: not a
        // cut of this store's record.
        let mut other = valid;
        other[0] = b'@';
        other[124..].fill(u8::MAX);
        let mut f = FakeJournalFlash::with_unit(256);
        place(&mut f, 0, &other);
        assert_eq!(scan(&mut f, layout, 0), Found::Unmarked);

        // The marker, but a bit gone from the body.
        let mut torn = valid;
        torn[40] = 0x7f;
        place(&mut f, 0, &torn);
        assert_eq!(scan(&mut f, layout, 0), Found::Torn);

        // Checked, but another store's magic or length, or fields the store
        // refuses.
        for (at, value) in [(0, b'Q'), (10, 64), (100, 1)] {
            let mut p = valid;
            p[at] = value;
            let crc = crc32(&p[..120]);
            p[120..124].copy_from_slice(&crc.to_le_bytes());
            place(&mut f, 0, &p);
            assert_eq!(scan(&mut f, layout, 0), Found::Invalid, "byte {at}");
        }

        // An older version, checked: superseded, never decoded.
        let mut p = valid;
        p[8] = 2;
        let crc = crc32(&p[..120]);
        p[120..124].copy_from_slice(&crc.to_le_bytes());
        place(&mut f, 0, &p);
        assert_eq!(scan(&mut f, layout, 0), Found::Older);

        // A newer version, checked.
        let mut p = valid;
        p[8] = 4;
        let crc = crc32(&p[..120]);
        p[120..124].copy_from_slice(&crc.to_le_bytes());
        place(&mut f, 0, &p);
        assert_eq!(scan(&mut f, layout, 0), Found::Future);

        // Written bytes after the record: only a unit read whole sees them.
        let mut f = FakeJournalFlash::with_unit(256);
        place(&mut f, 0, &valid);
        f.bytes[0][200] = 0;
        let expected = if layout.extent == Extent::Unit {
            Found::Invalid
        } else {
            Found::Valid((2, 9))
        };
        assert_eq!(scan(&mut f, layout, 0), expected);
        f.bytes[0][..RECORD_BYTES].fill(u8::MAX);
        let expected = if layout.extent == Extent::Unit {
            Found::Invalid
        } else {
            Found::Erased
        };
        assert_eq!(scan(&mut f, layout, 0), expected);

        // A failed read is no state at all.
        let mut f = FakeJournalFlash::with_unit(256);
        f.fail_call = 1;
        assert_eq!(units(&mut f, layout).scan(0, decode), None);
    }
}

#[test]
fn the_newest_valid_record_wins_unless_a_newer_version_is_present() {
    let v = |generation| Found::Valid(generation);
    let same = |&g: &u64| g;

    assert_eq!(newest(v(1), v(2), same), Pick::Newest(1, 2));
    assert_eq!(newest(v(3), v(2), same), Pick::Newest(0, 3));
    assert_eq!(newest(v(2), v(2), same), Pick::Newest(0, 2), "a tie");
    for other in [
        Found::Erased,
        Found::Partial,
        Found::Unmarked,
        Found::Torn,
        Found::Invalid,
    ] {
        assert_eq!(newest(v(5), other, same), Pick::Newest(0, 5));
        assert_eq!(newest(other, v(5), same), Pick::Newest(1, 5));
        assert_eq!(newest(other, Found::Erased, same), Pick::Nothing);
        assert_eq!(newest(Found::Future, other, same), Pick::Future);
    }
    assert_eq!(newest(v(9), Found::Future, same), Pick::Future);

    assert_eq!(target(0, 1), 0, "an empty store starts at unit 0");
    assert_eq!((target(4, 0), target(4, 1)), (1, 0));
}

/// Units holding `history` alternating records, as saves leave them; the
/// last record's unit.
fn saved(f: &mut FakeJournalFlash, layout: &'static Layout, history: u8) -> u32 {
    let mut slot = 0;
    for generation in 1..=u64::from(history) {
        slot = target(generation - 1, slot);
        let p = record(layout, generation, generation as u8);
        assert!(units(f, layout).write(slot, &p, Prepare::Erase));
    }
    slot
}

/// After a write cut short or failed, on units that held `before`: the
/// other unit is untouched, the written one holds its old record, a state a
/// cut leaves, or the new record, and the newest pick is the old record or
/// the new one.
fn check_cut(
    f: &mut FakeJournalFlash,
    layout: &'static Layout,
    before: &FakeJournalFlash,
    written: u32,
    history: u8,
    case: &str,
) {
    let (w, other) = (written as usize, 1 - written as usize);
    assert_eq!(
        f.bytes[other], before.bytes[other],
        "{case}: the other unit"
    );

    let new = (u64::from(history) + 1, history + 1);
    let was = scan(&mut before.clone(), layout, written);
    let found = scan(f, layout, written);
    assert!(
        matches!(
            found,
            Found::Erased | Found::Partial | Found::Unmarked | Found::Torn
        ) || found == Found::Valid(new)
            || found == was,
        "{case}: {found:?}"
    );
    if found == was {
        assert_eq!(f.bytes[w], before.bytes[w], "{case}: an unchanged unit");
    }

    let (a, b) = (scan(f, layout, 0), scan(f, layout, 1));
    // A first save cut anywhere, in any order, holds nothing committed.
    if history == 0 && found != Found::Valid(new) {
        assert!(unwritten(&a, &b), "{case}: a cut first save is unwritten");
    }
    let old = (history > 0).then(|| (u64::from(history), history));
    match newest(a, b, |&(generation, _)| generation) {
        Pick::Newest(_, value) => assert!(Some(value) == old || value == new, "{case}: {value:?}"),
        Pick::Nothing => assert_eq!(old, None, "{case}: the old record was lost"),
        Pick::Future => panic!("{case}: a newer version from nowhere"),
    }
}

#[test]
fn a_power_cut_at_every_mutation_leaves_the_old_record_or_the_new() {
    for order in ORDERS {
        for layout in [&UNIT, &RECORD] {
            for unit in SIZES {
                power_cuts(order, layout, unit);
            }
        }
    }
}

fn power_cuts(order: ProgramOrder, layout: &'static Layout, unit: usize) {
    for history in 0..3u8 {
        let mut base = FakeJournalFlash::with_unit(unit).ordered(order);
        let last = saved(&mut base, layout, history);
        let written = target(u64::from(history), last);
        let next = record(layout, u64::from(history) + 1, history + 1);

        let mut probe = base.clone();
        let start = probe.mutations;
        assert!(units(&mut probe, layout).write(written, &next, Prepare::Erase));
        let span = i64::from(probe.mutations - start);
        if order == ProgramOrder::Bytes {
            assert_eq!(span as usize, unit + RECORD_BYTES, "one erase, one record");
        }

        for cut in 0..=span {
            let mut f = base.clone();
            f.budget = cut;
            let ok = units(&mut f, layout).write(written, &next, Prepare::Erase);
            f.budget = -1;

            // The fake reports a cut even after its last mutation.
            let case = format!("{order:?} {layout:?} unit {unit} history {history} cut {cut}");
            assert!(!ok, "{case}");
            check_cut(&mut f, layout, &base, written, history, &case);

            // The written unit takes the next write whatever it holds.
            assert!(units(&mut f, layout).write(written, &next, Prepare::Erase));
            assert_eq!(
                scan(&mut f, layout, written),
                Found::Valid((u64::from(history) + 1, history + 1))
            );
        }
    }
}

#[test]
fn a_failure_at_every_call_leaves_the_old_record_or_the_new() {
    for layout in [&UNIT, &RECORD] {
        for unit in SIZES.into_iter().chain([SECTOR]) {
            for history in 0..3u8 {
                let mut base = FakeJournalFlash::with_unit(unit);
                let last = saved(&mut base, layout, history);
                let written = target(u64::from(history), last);
                let next = record(layout, u64::from(history) + 1, history + 1);

                let mut probe = base.clone();
                let start = probe.calls;
                assert!(units(&mut probe, layout).write(written, &next, Prepare::Erase));
                let calls = probe.calls - start;
                // Erase, the blank check (record, then the tail in chunks),
                // body, verify, marker, verify.
                assert!(calls >= 6);

                for failure in 1..=calls {
                    let mut f = base.clone();
                    f.fail_call = f.calls + failure;
                    let ok = units(&mut f, layout).write(written, &next, Prepare::Erase);
                    f.fail_call = 0;

                    let case = format!("{layout:?} unit {unit} history {history} call {failure}");
                    assert!(!ok, "{case}: a failed call fails the write");
                    check_cut(&mut f, layout, &base, written, history, &case);
                }
            }
        }
    }
}

#[test]
fn a_write_that_did_not_land_is_caught_by_its_checks() {
    for layout in [&UNIT, &RECORD] {
        let mut f = FakeJournalFlash::with_unit(256);
        let last = saved(&mut f, layout, 2);
        let next = record(layout, 3, 3);

        f.silent_program = true;
        assert!(!units(&mut f, layout).write(1 - last, &next, Prepare::Erase));
        f.silent_program = false;

        // An erase that did nothing leaves the old record, which the blank
        // check refuses to program over.
        f.silent_erase = true;
        place(&mut f, 1 - last as usize, &record(layout, 1, 1));
        assert!(!units(&mut f, layout).write(1 - last, &next, Prepare::Erase));
        assert_eq!(scan(&mut f, layout, 1 - last), Found::Valid((1, 1)));
        f.silent_erase = false;

        // Nor does a first provisioning program over anything.
        assert!(!units(&mut f, layout).write(1 - last, &next, Prepare::Erased));
    }
}

#[test]
fn unbound_units_refuse_everything_and_a_clear_erases_both() {
    let unbound: DualSlot<FlashIo> = DualSlot::unbound(&UNIT);
    assert_eq!(unbound.scan(0, decode), None);
    assert!(!unbound.writable() && !unbound.clear());
    assert!(!unbound.write(0, &record(&UNIT, 1, 1), Prepare::Erase));

    let mut f = FakeJournalFlash::with_unit(SECTOR);
    saved(&mut f, &UNIT, 2);
    f.bytes[1][4000] = 0;
    assert!(units(&mut f, &UNIT).clear());
    assert!(f.bytes.iter().flatten().all(|&b| b == u8::MAX));
}

#[test]
fn only_erased_units_or_a_cut_first_save_are_unwritten() {
    let found = [
        Found::Erased,
        Found::Valid(1u64),
        Found::Future,
        Found::Older,
        Found::Partial,
        Found::Unmarked,
        Found::Torn,
        Found::Invalid,
    ];
    for a in found {
        for b in found {
            let blank = |f: Found<u64>| f == Found::Erased;
            let expected = (blank(a) && blank(b))
                || (a == Found::Partial && blank(b))
                || (blank(a) && b == Found::Partial);
            assert_eq!(unwritten(&a, &b), expected, "{a:?} {b:?}");
        }
    }
}

/// A record of this store in version `major.minor` and `len` bytes: the
/// generation and payload this build knows, then `len - 32` bytes it
/// doesn't, none of them zero.
fn versioned(major: u8, minor: u8, len: usize, generation: u64, value: u8) -> Vec<u8> {
    let mut p = vec![0u8; len];
    p[..8].copy_from_slice(b"PLFTEST1");
    p[8] = major;
    p[9] = minor;
    p[10..12].copy_from_slice(&u16::try_from(len).unwrap().to_le_bytes());
    p[12..20].copy_from_slice(&generation.to_le_bytes());
    p[20] = value;
    for (i, b) in p[24..len - 8].iter_mut().enumerate() {
        *b = (i as u8) | 0x80;
    }
    let crc = crc32(&p[..len - 8]);
    p[len - 8..len - 4].copy_from_slice(&crc.to_le_bytes());
    p[len - 4..].copy_from_slice(b"CMIT");
    p
}

/// What `layout` reads in unit `slot`, with the version the decoder saw.
fn scan_versioned(
    f: &mut FakeJournalFlash,
    layout: &'static Layout,
    slot: u32,
) -> Found<(Version, u64, u8)> {
    units(f, layout)
        .scan(slot, |version, p| {
            decode(version, p).map(|(generation, value)| (version, generation, value))
        })
        .expect("the fake flash reads")
}

#[test]
fn a_newer_minor_is_read_for_the_fields_this_build_knows() {
    for layout in [&UNIT, &RECORD] {
        // Longer than the build's record, or the same length with its
        // spare room used: either way the decoder sees the unknown bytes as
        // zero.
        // A store read whole never grows: a cut erase over a longer record
        // would leave written bytes past its own.
        for len in [RECORD_BYTES, 256, 1024] {
            let mut f = FakeJournalFlash::with_unit(1024);
            place(&mut f, 0, &versioned(3, 1, len, 7, 9));
            let expected = if len == RECORD_BYTES || layout.extent == Extent::Record {
                Found::Valid((Version::new(3, 1), 7, 9))
            } else {
                Found::Invalid
            };
            assert_eq!(
                scan_versioned(&mut f, layout, 0),
                expected,
                "{layout:?} {len}"
            );
        }

        // Its checksum is the one at its own length.
        let mut f = FakeJournalFlash::with_unit(512);
        let mut p = versioned(3, 1, 256, 7, 9);
        p[200] ^= 1;
        place(&mut f, 0, &p);
        assert!(!scan(&mut f, layout, 0).is_valid());

        // A newer minor never shrinks the record.
        let mut f = FakeJournalFlash::with_unit(512);
        place(&mut f, 0, &versioned(3, 1, 64, 7, 9));
        assert_eq!(scan(&mut f, layout, 0), Found::Invalid);

        // A length past the unit is no length at all.
        let mut f = FakeJournalFlash::with_unit(256);
        let mut p = versioned(3, 1, 256, 7, 9);
        p[10..12].copy_from_slice(&512u16.to_le_bytes());
        place(&mut f, 0, &p);
        assert!(!scan(&mut f, layout, 0).is_valid());

        // A newer major is never decoded, whatever its length.
        for len in [RECORD_BYTES, 256] {
            let mut f = FakeJournalFlash::with_unit(512);
            place(&mut f, 0, &versioned(4, 0, len, 7, 9));
            assert_eq!(scan(&mut f, layout, 0), Found::Future, "{len}");
        }
    }

    // Read alone, a longer record ignores what follows it.
    let mut f = FakeJournalFlash::with_unit(512);
    place(&mut f, 0, &versioned(3, 1, 256, 7, 9));
    f.bytes[0][300] = 0;
    assert_eq!(scan(&mut f, &RECORD, 0), Found::Valid((7, 9)));
}

/// This build's record grown by a minor: fields at 12..21 and 24..28.
const GROWN: Layout = Layout::new(*b"PLFTEST1", Version::new(3, 2), 28, Extent::Record);
/// A build that moved to major 4 and still reads major 3.
const MOVED: Layout = Layout {
    oldest_major: 3,
    ..Layout::new(*b"PLFTEST1", Version::new(4, 0), 24, Extent::Unit)
};

#[test]
fn an_older_minor_or_a_previous_major_reaches_the_decoder() {
    // A shorter record of an older minor: what it lacks reads as zero.
    let mut short: [u8; 64] = UNIT.record();
    put_u64::<12, _>(&mut short, 5);
    short[20] = 6;
    seal(&mut short);
    let mut f = FakeJournalFlash::with_unit(256);
    place(&mut f, 0, &short);
    let found = units(&mut f, &GROWN)
        .scan(0, |version, p: &[u8; RECORD_BYTES]| {
            p[64..].iter().all(|&b| b == 0).then_some((version, p[20]))
        })
        .unwrap();
    assert_eq!(found, Found::Valid((Version::new(3, 0), 6)));

    // The same length: its spare room must still be zero.
    place(&mut f, 0, &record(&UNIT, 5, 6));
    assert_eq!(
        scan_versioned(&mut f, &GROWN, 0),
        Found::Valid((Version::new(3, 0), 5, 6))
    );
    let mut f = FakeJournalFlash::with_unit(256);
    place(&mut f, 0, &versioned(3, 0, RECORD_BYTES, 5, 6));
    assert_eq!(scan(&mut f, &GROWN, 0), Found::Invalid);

    // The build's own minor in another length is no record of it.
    let mut f = FakeJournalFlash::with_unit(256);
    let mut p = versioned(3, 0, 64, 5, 6);
    p[24..56].fill(0);
    let crc = crc32(&p[..56]);
    p[56..60].copy_from_slice(&crc.to_le_bytes());
    place(&mut f, 0, &p);
    assert_eq!(scan(&mut f, &UNIT, 0), Found::Invalid);
    assert_eq!(scan(&mut f, &RECORD, 0), Found::Invalid);

    // A previous major the layout still reads goes to its decoder with
    // its version; one older than that is superseded.
    let mut f = FakeJournalFlash::with_unit(256);
    place(&mut f, 0, &record(&UNIT, 5, 6));
    assert_eq!(
        scan_versioned(&mut f, &MOVED, 0),
        Found::Valid((Version::new(3, 0), 5, 6))
    );
    let only_four = Layout {
        oldest_major: 4,
        ..MOVED
    };
    let only_four: &'static Layout = Box::leak(Box::new(only_four));
    assert_eq!(scan(&mut f, only_four, 0), Found::Older);
}

#[test]
fn a_rewrite_keeps_a_newer_minors_bytes() {
    for layout in [&UNIT, &RECORD] {
        let grows = layout.extent == Extent::Record;
        let lens = [RECORD_BYTES, 256, 1024].into_iter();
        for len in lens.filter(|&len| grows || len == RECORD_BYTES) {
            let mut f = FakeJournalFlash::with_unit(1024);
            let old = versioned(3, 1, len, 7, 9);
            place(&mut f, 0, &old);
            assert!(units(&mut f, layout).replace(1, Some(0), &record(layout, 8, 10)));

            // The known fields are the build's, the rest the newer minor's,
            // under its version and length.
            let written = &f.bytes[1][..len];
            assert_eq!(&written[..12], &old[..12], "{layout:?} {len}");
            assert_eq!(&written[12..24], &record(layout, 8, 10)[12..24]);
            assert_eq!(&written[24..len - 8], &old[24..len - 8]);
            assert_eq!(
                written[len - 8..len - 4],
                crc32(&written[..len - 8]).to_le_bytes()
            );
            assert_eq!(&written[len - 4..], b"CMIT");
            assert!(f.bytes[1][len..].iter().all(|&b| b == u8::MAX));
            assert_eq!(
                scan_versioned(&mut f, layout, 1),
                Found::Valid((Version::new(3, 1), 8, 10))
            );
            assert_eq!(f.bytes[0][..len], old[..], "the source untouched");

            // Rewritten again from the carried record, nothing changes but
            // the fields this build knows.
            assert!(units(&mut f, layout).replace(0, Some(1), &record(layout, 9, 11)));
            assert_eq!(&f.bytes[0][24..len - 8], &old[24..len - 8]);
            assert_eq!(scan(&mut f, layout, 0), Found::Valid((9, 11)));
        }

        // From the build's own minor, an older one, or none at all, a
        // rewrite is a plain write.
        for from in [None, Some(0)] {
            let mut f = FakeJournalFlash::with_unit(256);
            place(&mut f, 0, &record(layout, 1, 1));
            assert!(units(&mut f, layout).replace(1, from, &record(layout, 2, 2)));
            assert_eq!(f.bytes[1][..RECORD_BYTES], record(layout, 2, 2));
        }
    }
}

/// Unit 0 holding a newer minor of `len` bytes, and the carried rewrite
/// into unit 1 as it lands whole.
fn carried_base(
    layout: &'static Layout,
    len: usize,
) -> (FakeJournalFlash, FakeJournalFlash, [u8; RECORD_BYTES]) {
    let mut base = FakeJournalFlash::with_unit(512);
    place(&mut base, 0, &versioned(3, 1, len, 7, 9));
    let next = record(layout, 8, 10);
    let mut done = base.clone();
    assert!(units(&mut done, layout).replace(1, Some(0), &next));
    (base, done, next)
}

/// After a carried rewrite cut short or failed: unit 0 untouched, unit 1
/// erased, a cut or the whole new record, and the pick the old or the new.
fn check_carried(
    f: &mut FakeJournalFlash,
    layout: &'static Layout,
    base: &FakeJournalFlash,
    done: &FakeJournalFlash,
    case: &str,
) {
    assert_eq!(f.bytes[0], base.bytes[0], "{case}: the source");
    let found = scan(f, layout, 1);
    assert!(
        matches!(
            found,
            Found::Erased | Found::Partial | Found::Unmarked | Found::Torn
        ) || (found == Found::Valid((8, 10)) && f.bytes[1] == done.bytes[1]),
        "{case}: {found:?}"
    );
    let pick = newest(scan(f, layout, 0), found, |&(generation, _)| generation);
    assert!(
        matches!(pick, Pick::Newest(0, (7, 9)) | Pick::Newest(1, (8, 10))),
        "{case}: {pick:?}"
    );
}

#[test]
fn a_carried_rewrite_cut_or_failed_leaves_the_old_record_or_the_new() {
    for (layout, len) in [
        (&UNIT, RECORD_BYTES),
        (&RECORD, RECORD_BYTES),
        (&RECORD, 256),
    ] {
        let (base, done, next) = carried_base(layout, len);

        let mut probe = base.clone();
        let start = probe.mutations;
        assert!(units(&mut probe, layout).replace(1, Some(0), &next));
        let span = i64::from(probe.mutations - start);
        assert_eq!(span as usize, 512 + len, "one erase, one record");
        for cut in 0..=span {
            let mut f = base.clone();
            f.budget = cut;
            assert!(!units(&mut f, layout).replace(1, Some(0), &next));
            f.budget = -1;
            let case = format!("{layout:?} {len} cut {cut}");
            check_carried(&mut f, layout, &base, &done, &case);
        }

        let mut probe = base.clone();
        let start = probe.calls;
        assert!(units(&mut probe, layout).replace(1, Some(0), &next));
        let calls = probe.calls - start;
        for failure in 1..=calls {
            let mut f = base.clone();
            f.fail_call = f.calls + failure;
            assert!(!units(&mut f, layout).replace(1, Some(0), &next));
            f.fail_call = 0;
            let case = format!("{layout:?} {len} call {failure}");
            check_carried(&mut f, layout, &base, &done, &case);
        }

        // A program that lands nothing is caught before the marker.
        let mut f = base.clone();
        f.silent_program = true;
        assert!(!units(&mut f, layout).replace(1, Some(0), &next));
    }
}

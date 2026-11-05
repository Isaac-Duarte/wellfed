//! KM0's trial report and boot count: the `PLTRL1` text the KM0 glue prints,
//! written without `core::fmt`, must stay byte for byte what the host tools
//! parse, and the boot count must leave the trial's bits alone.

use wellfed_core::record::{Event, Record, Trial};
use wellfed_hal_common::trial::{
    BOOT_MASK, Decision, LIMIT, Line, Reset, STORM_LIMIT, boot_counted, report,
};

fn line(write: impl FnOnce(&mut Line<'_>)) -> String {
    let mut buffer = [0xaa_u8; 128];
    let mut line = Line::new(&mut buffer);
    write(&mut line);
    let written = line.as_bytes().to_vec();

    // The glue prints the buffer as a C string: the NUL ends the text.
    let end = buffer.iter().position(|&byte| byte == 0).expect("a NUL");
    assert_eq!(&buffer[..end], written.as_slice());
    String::from_utf8(written).expect("ASCII")
}

fn decision(decision: Decision) -> String {
    line(|line| decision.report(line))
}

#[test]
fn each_decision_prints_the_line_the_glue_printed() {
    let cases = [
        (Decision::NotBank1 { resets: 0 }, "PLTRL1 bank=0 resets=0\n"),
        (Decision::NotBank1 { resets: 4 }, "PLTRL1 bank=0 resets=4\n"),
        (
            Decision::Confirmed { resets: 0 },
            "PLTRL1 confirmed=1 resets=0\n",
        ),
        (
            Decision::Confirmed { resets: 3 },
            "PLTRL1 confirmed=1 resets=3\n",
        ),
        (Decision::Counted(1), "PLTRL1 confirmed=0 boot=1 limit=3\n"),
        (Decision::Counted(3), "PLTRL1 confirmed=0 boot=3 limit=3\n"),
        (Decision::GiveWay, ""),
        (
            Decision::NoFallback(3),
            "PLTRL1 error fallback=invalid boot=3\n",
        ),
        (Decision::Held { resets: 5 }, "PLTRL1 storm=5 held=1\n"),
        (Decision::Held { resets: 15 }, "PLTRL1 storm=15 held=1\n"),
        (
            Decision::StormGiveWay { resets: 5 },
            "PLTRL1 storm=5 limit=5\n",
        ),
    ];

    for (case, text) in cases {
        assert_eq!(decision(case), text, "{case:?}");
    }
    // The glue printed these limits as literals.
    assert_eq!((LIMIT, STORM_LIMIT), (3, 5));
}

#[test]
fn only_a_give_way_tells_the_glue_to_give_way() {
    let giving = [Decision::GiveWay, Decision::StormGiveWay { resets: 5 }];
    let staying = [
        Decision::NotBank1 { resets: 0 },
        Decision::Confirmed { resets: 1 },
        Decision::Counted(2),
        Decision::NoFallback(3),
        Decision::Held { resets: 5 },
    ];

    assert!(giving.iter().all(|decision| decision.gives_way()));
    assert!(!staying.iter().any(|decision| decision.gives_way()));
}

#[test]
fn a_brownout_boot_reports_the_register_it_found_first() {
    let found = 0x0a15_0007;

    let text = line(|line| {
        report(
            Reset::Brownout,
            found,
            Decision::Confirmed { resets: 1 },
            line,
        )
    });
    assert_eq!(
        text,
        "PLTRL1 brownout=1 reg=0a150007\nPLTRL1 confirmed=1 resets=1\n"
    );

    let text = line(|line| report(Reset::Brownout, 0xdead_beef, Decision::GiveWay, line));
    assert_eq!(text, "PLTRL1 brownout=1 reg=deadbeef\n");

    let text = line(|line| report(Reset::Other, found, Decision::Counted(2), line));
    assert_eq!(text, "PLTRL1 confirmed=0 boot=2 limit=3\n");
}

#[test]
fn the_lines_parse_as_the_host_tools_read_them() {
    let cases = [
        (Decision::Confirmed { resets: 2 }, Trial::Confirmed),
        (
            Decision::Counted(2),
            Trial::Booting {
                boot: 2,
                limit: Some(3),
            },
        ),
        (
            Decision::Held { resets: 6 },
            Trial::Storm {
                resets: 6,
                held: true,
            },
        ),
        (
            Decision::StormGiveWay { resets: 5 },
            Trial::Storm {
                resets: 5,
                held: false,
            },
        ),
    ];

    for (case, trial) in cases {
        let text = decision(case);
        let record = Record::parse(text.trim_end()).expect("a record");
        assert_eq!(record.event(), Some(Event::Trial(trial)), "{text}");
    }
}

#[test]
fn numbers_print_as_c_prints_them() {
    let decimals = [
        (0, "0"),
        (7, "7"),
        (10, "10"),
        (305, "305"),
        (1_000_000_000, "1000000000"),
        (u32::MAX, "4294967295"),
    ];
    for (value, text) in decimals {
        assert_eq!(line(|line| line.decimal(value)), text);
    }

    assert_eq!(line(|line| line.hex8(0)), "00000000");
    assert_eq!(line(|line| line.hex8(0x0123_abcd)), "0123abcd");
    assert_eq!(line(|line| line.hex8(u32::MAX)), "ffffffff");
}

#[test]
fn a_short_buffer_keeps_its_nul_and_drops_the_rest() {
    let mut buffer = [0xaa_u8; 8];
    let mut line = Line::new(&mut buffer);
    line.text("PLTRL1 ");
    line.decimal(12345);
    assert_eq!(line.as_bytes(), b"PLTRL1 ");
    assert_eq!(buffer, *b"PLTRL1 \0");

    let mut one = [0xaa_u8; 1];
    let mut line = Line::new(&mut one);
    line.text("x");
    assert!(line.as_bytes().is_empty());
    assert_eq!(one, [0]);

    let mut none: [u8; 0] = [];
    let mut line = Line::new(&mut none);
    line.hex8(1);
    assert!(line.as_bytes().is_empty());
}

#[test]
fn the_boot_count_keeps_the_trial_bits() {
    assert_eq!(boot_counted(0), (1, 1));
    assert_eq!(boot_counted(0x8a15_0041), (0x8a15_0042, 0x42));
    // The count wraps within its 16 bits and never carries into the trial's.
    assert_eq!(boot_counted(0x0a15_ffff), (0x0a15_0000, 0));
    assert_eq!(boot_counted(u32::MAX), (!BOOT_MASK, 0));
}

#[test]
fn the_longest_report_fits_the_glue_buffer() {
    // `wellfed_trial_step` in `firstboot_km0.c` passes a 96-byte buffer.
    let longest = line(|line| {
        report(
            Reset::Brownout,
            u32::MAX,
            Decision::NoFallback(u32::MAX),
            line,
        );
    });

    assert_eq!(
        longest,
        "PLTRL1 brownout=1 reg=ffffffff\nPLTRL1 error fallback=invalid boot=4294967295\n"
    );
    assert_eq!(longest.len() + 1, 78);
    assert!(longest.len() < 96);
}

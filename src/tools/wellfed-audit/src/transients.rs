//! Logic-analyzer captures of the motor driver, checked for any drive.
//!
//! It reads a `wellfed-bench transients`
//! session: raw sigrok bytes (one byte per sample, bit n = channel Dn) plus a
//! manifest naming the INA, INB, OUTA and OUTB channels. The TC118AH drives
//! the motor only when its inputs differ (forward H/L, reverse L/H); L/L is
//! standby and H/H is brake. Its outputs differ only while driving.
//!
//! Per cycle, a pass needs:
//! - the capture to start with every probed line LOW (feeder unpowered), so
//!   the power-on itself was recorded;
//! - no sample with INA != INB, and none with OUTA != OUTB, but for the
//!   supply ramp's skews (below);
//! - boot mode: the inputs reach H/H (stopped startup's brake) and stay there
//!   to the end, once the ramp is over; loader mode: the inputs never leave
//!   L/L.
//!
//! **The supply ramp.** Within the first millisecond after power-on (the
//! first sample any probed line leaves LOW), before any code runs, the
//! analyzer resolves lines crossing its threshold one at a time, and the
//! recorded sessions show single skews of up to 4.5 µs between them. The
//! pass rule the motor driver's evidence applies excludes that millisecond,
//! and a differing read counts as drive only when the levels are firm. So a
//! disagreement that starts inside [`RAMP_US`] of power-on and lasts at most
//! [`SKEW_US`] is reported as a `ramp_skew`, kept out of the outcome, and a
//! brief return to standby inside the ramp does not leave the brake. A
//! longer disagreement, even inside the ramp, is drive. Whether the skews
//! belong to the probe and the ramp, not the driver, is an open bench check
//! (a scope on the inputs); until then the report counts them. An
//! outputs-only session has no power-on edge of its own, so it takes no ramp.
//!
//! A session may probe only OUTA and OUTB, at the motor's terminals, which
//! take ordinary hooks where the driver's 1.27 mm pins do not. The outputs
//! differ only while the motor is driven, so the drive check stands. Standby
//! leaves the outputs undriven, at whatever level the analyzer's inputs float
//! to, so the brake rules are not checked, and the report says so. The
//! capture must still start with both outputs LOW, as an unpowered driver's
//! read, so a capture that began after power-on is refused.
//!
//! Every input or output disagreement is reported with its start and length,
//! however short; one sample is a sample period long.

use std::fs;
use std::path::{Component, Path};

use serde::Serialize;
use wellfed_bench_formats::transients::{TransientMode, TransientsManifest};
use wellfed_formats::require;

use crate::{Result, outcomes, round};

/// A transients capture refused before any cycle is checked.
#[derive(Debug, thiserror::Error)]
pub enum TransientsError {
    #[error("Unsupported transients manifest")]
    Schema { version: u32 },
    #[error("Unsupported transients manifest")]
    Samplerate,
    /// A cycle file must be a plain name inside the capture directory, and a
    /// regular file there, not a link to somewhere else.
    #[error("Cycle file {file:?} must be a regular file named inside the capture")]
    CycleFile { file: String },
    #[error(
        "Manifest needs distinct channels 0-7 for INA, INB, OUTA and OUTB, or for OUTA and OUTB alone"
    )]
    Channels,
    /// The manifest records each cycle's length, not its hash, so only a
    /// change of size is caught.
    #[error("{file} changed size since capture")]
    SizeChanged { file: String },
}

const LINES: [&str; 4] = ["ina", "inb", "outa", "outb"];
const LIMIT: &str = "Digital levels at the analyzer threshold (about 1.4 V for fx2lafw clones), sampled at the recorded \
    rate; a pulse shorter than one sample period can be missed. Undriven driver outputs (standby) float to the \
    analyzer's input level, which read HIGH on the tested clone.";
const OUTPUTS_ONLY: &str = "INA and INB were not probed: the brake state and the power-on edge are not seen, \
    so power-on is placed only by the bench's timing (power_on_after_launch_s), and no ramp skew is excused.";
/// How long after power-on the supply ramp lasts, in microseconds: the
/// motor driver's evidence takes every sample after its first millisecond.
const RAMP_US: f64 = 1000.0;
/// The longest disagreement inside the ramp that is a threshold skew rather
/// than drive, in microseconds: the longest the recorded sessions show is
/// 4.5 µs.
const SKEW_US: f64 = 5.0;

outcomes! {
    /// A cycle's outcome, or the session's (`pass` only when every cycle passed).
    pub enum TransientOutcome {
        Pass => "pass",
        Empty => "transients_empty",
        LateStart => "transients_late_start",
        InputDrive => "transients_input_drive",
        OutputDrive => "transients_output_drive",
        NoBrake => "transients_no_brake",
        LeftBrake => "transients_left_brake",
        LoaderDriven => "transients_loader_driven",
        Fail => "transients_fail",
    }
}

/// Which driver lines a session probed, from the manifest's channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Probed {
    Driver,
    Outputs,
}

/// Analyzer channels: INA and INB when probed, and always OUTA and OUTB.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Probes {
    inputs: Option<[u8; 2]>,
    outputs: [u8; 2],
}

impl Probes {
    fn probed(&self) -> Probed {
        if self.inputs.is_some() {
            Probed::Driver
        } else {
            Probed::Outputs
        }
    }
}

/// (value, start, length) for each run of `byte & mask` in the capture.
pub(crate) fn runs(data: &[u8], mask: u8) -> Vec<(u8, usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut current: Option<u8> = None;
    for (index, byte) in data.iter().enumerate() {
        let value = byte & mask;
        if Some(value) != current {
            if let Some(previous) = current {
                out.push((previous, start, index - start));
            }
            current = Some(value);
            start = index;
        }
    }

    if let Some(previous) = current {
        out.push((previous, start, data.len() - start));
    }
    out
}

/// Which way differing inputs drive the motor.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Direction {
    Forward,
    Reverse,
}

/// The driver output that was high while the other was low.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Output {
    Outa,
    Outb,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Drive {
    Direction(Direction),
    High(Output),
}

/// One run of differing inputs or outputs.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct DriveEvent {
    start_us: f64,
    length_us: f64,
    #[serde(flatten)]
    drive: Drive,
}

/// What a non-empty capture's timeline showed.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Timeline {
    /// When the inputs first reached H/H, if they did.
    brake_at_s: Option<f64>,
    transitions: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CycleCheck {
    samples: usize,
    seconds: f64,
    input_drive: Vec<DriveEvent>,
    output_drive: Vec<DriveEvent>,
    /// Disagreements inside the supply ramp short enough to be threshold
    /// skews, kept out of the outcome.
    ramp_skew: Vec<DriveEvent>,
    #[serde(flatten)]
    timeline: Option<Timeline>,
    outcome: TransientOutcome,
}

pub(crate) fn check_cycle(
    data: &[u8],
    probes: &Probes,
    mode: TransientMode,
    samplerate: f64,
) -> CycleCheck {
    let bit = |channel: u8| 1u8 << channel;
    let [outa, outb] = probes.outputs.map(bit);
    let input_bits = probes.inputs.map(|pair| pair.map(bit));
    let mask = outa | outb | input_bits.map_or(0, |[ina, inb]| ina | inb);
    let us = 1e6 / samplerate;
    let mut check = CycleCheck {
        samples: data.len(),
        seconds: round(data.len() as f64 / samplerate, 6),
        input_drive: Vec::new(),
        output_drive: Vec::new(),
        ramp_skew: Vec::new(),
        timeline: None,
        outcome: TransientOutcome::Empty,
    };
    if data.is_empty() {
        return check;
    }

    let timeline = runs(data, mask);
    let on = |value: u8, bit: u8| value & bit != 0;
    // Power-on, where the inputs show it: the first sample any probed line
    // leaves LOW, in a capture that starts LOW.
    let power_on = timeline
        .get(1)
        .filter(|_| input_bits.is_some() && timeline[0].0 == 0)
        .map(|&(_, start, _)| start as f64 * us);
    let ramp_end = power_on.map(|at| at + RAMP_US);
    let in_ramp =
        |at: f64| power_on.is_some_and(|on| at >= on) && ramp_end.is_some_and(|end| at < end);
    for &(value, start, length) in &timeline {
        let event = |drive| DriveEvent {
            start_us: round(start as f64 * us, 3),
            length_us: round(length as f64 * us, 3),
            drive,
        };
        let skew = in_ramp(start as f64 * us) && length as f64 * us <= SKEW_US;
        if let Some([ina, inb]) = input_bits
            && on(value, ina) != on(value, inb)
        {
            let drive = event(Drive::Direction(if on(value, ina) {
                Direction::Forward
            } else {
                Direction::Reverse
            }));
            if skew {
                check.ramp_skew.push(drive);
            } else {
                check.input_drive.push(drive);
            }
        }
        if on(value, outa) != on(value, outb) {
            let drive = event(Drive::High(if on(value, outa) {
                Output::Outa
            } else {
                Output::Outb
            }));
            if skew {
                check.ramp_skew.push(drive);
            } else {
                check.output_drive.push(drive);
            }
        }
    }

    // The outputs alone cannot tell brake from standby, so without the inputs
    // there are no input states and no brake time.
    let inputs: Vec<(bool, bool, usize)> = input_bits.map_or_else(Vec::new, |[ina, inb]| {
        timeline
            .iter()
            .map(|(v, s, _)| (on(*v, ina), on(*v, inb), *s))
            .collect()
    });
    let brake_at = inputs.iter().find(|(a, b, _)| *a && *b).map(|(_, _, s)| *s);
    check.timeline = Some(Timeline {
        brake_at_s: brake_at.map(|s| round(s as f64 / samplerate, 6)),
        transitions: timeline.len() - 1,
    });

    check.outcome = if timeline[0].0 != 0 {
        TransientOutcome::LateStart
    } else if !check.input_drive.is_empty() {
        TransientOutcome::InputDrive
    } else if !check.output_drive.is_empty() {
        TransientOutcome::OutputDrive
    } else if input_bits.is_none() {
        TransientOutcome::Pass
    } else if mode == TransientMode::Boot {
        let last = inputs.last().is_some_and(|(a, b, _)| *a && *b);
        match brake_at {
            // After the first brake the inputs must not return to standby,
            // once the supply ramp is over.
            Some(at) if last => {
                let settled = |s: usize| ramp_end.is_none_or(|end| s as f64 * us >= end);
                if inputs
                    .iter()
                    .any(|(x, y, s)| *s > at && settled(*s) && !(*x && *y))
                {
                    TransientOutcome::LeftBrake
                } else {
                    TransientOutcome::Pass
                }
            }
            _ => TransientOutcome::NoBrake,
        }
    } else if brake_at.is_some() {
        TransientOutcome::LoaderDriven
    } else {
        TransientOutcome::Pass
    };
    check
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Cycle {
    file: String,
    #[serde(flatten)]
    check: CycleCheck,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct Range {
    min: f64,
    max: f64,
}

/// The session's totals, printed without the per-cycle detail.
#[derive(Debug, Clone, Serialize)]
pub struct TransientsSummary {
    outcome: TransientOutcome,
    mode: TransientMode,
    probed: Probed,
    samplerate: u64,
    cycles: usize,
    passed: usize,
    input_drive_events: usize,
    output_drive_events: usize,
    /// Threshold skews inside the supply ramp, counted but not failed.
    ramp_skew_events: usize,
    /// The ramp and the longest skew it excuses, in microseconds.
    ramp_us: f64,
    skew_us: f64,
    brake_at_s: Option<Range>,
    limit: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    not_probed: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransientsReport {
    #[serde(flatten)]
    pub summary: TransientsSummary,
    per_cycle: Vec<Cycle>,
}

impl TransientsReport {
    pub fn passed(&self) -> bool {
        self.summary.outcome == TransientOutcome::Pass
    }
}

/// The probed lines' analyzer channels, distinct and each 0-7: all four
/// driver lines, or OUTA and OUTB alone.
fn probes(manifest: &TransientsManifest) -> Result<Probes> {
    let refused = || crate::Error::from(TransientsError::Channels);
    if manifest
        .channels
        .keys()
        .any(|name| !LINES.contains(&name.as_str()))
    {
        return Err(refused());
    }

    let channel = |name: &str| {
        manifest
            .channels
            .get(name)
            .and_then(|&c| u8::try_from(c).ok())
            .filter(|c| *c < 8)
    };
    let [Some(outa), Some(outb)] = [channel("outa"), channel("outb")] else {
        return Err(refused());
    };
    let inputs = match manifest.channels.len() {
        2 => None,
        4 => {
            let [Some(ina), Some(inb)] = [channel("ina"), channel("inb")] else {
                return Err(refused());
            };
            Some([ina, inb])
        }
        _ => return Err(refused()),
    };

    let mut distinct: Vec<u8> = inputs.into_iter().flatten().chain([outa, outb]).collect();
    let probed = distinct.len();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.len() != probed {
        return Err(refused());
    }
    Ok(Probes {
        inputs,
        outputs: [outa, outb],
    })
}

pub fn verify(capture: &Path) -> Result<TransientsReport> {
    let manifest: TransientsManifest = wellfed_formats::load(&capture.join("manifest.json"))?;
    require(manifest.schema_version == 1, || TransientsError::Schema {
        version: manifest.schema_version,
    })?;
    let probes = probes(&manifest)?;
    require(manifest.samplerate > 0, || TransientsError::Samplerate)?;
    let samplerate = manifest.samplerate as f64;

    let mut cycles = Vec::new();
    for entry in &manifest.cycles {
        let file = entry.file.as_str();
        let plain = matches!(
            Path::new(file).components().collect::<Vec<_>>()[..],
            [Component::Normal(_)]
        );
        require(
            plain && fs::symlink_metadata(capture.join(file)).is_ok_and(|meta| meta.is_file()),
            || TransientsError::CycleFile { file: file.into() },
        )?;
        let data = fs::read(capture.join(file))?;
        require(entry.bytes == data.len() as u64, || {
            TransientsError::SizeChanged { file: file.into() }
        })?;

        cycles.push(Cycle {
            file: file.into(),
            check: check_cycle(&data, &probes, manifest.mode, samplerate),
        });
    }

    let passed = cycles
        .iter()
        .filter(|c| c.check.outcome == TransientOutcome::Pass)
        .count();
    let brakes: Vec<f64> = cycles
        .iter()
        .filter_map(|c| c.check.timeline.as_ref()?.brake_at_s)
        .collect();
    let brake_at_s = (!brakes.is_empty()).then(|| Range {
        min: brakes.iter().copied().fold(f64::INFINITY, f64::min),
        max: brakes.iter().copied().fold(f64::NEG_INFINITY, f64::max),
    });

    let summary = TransientsSummary {
        outcome: if !cycles.is_empty() && passed == cycles.len() {
            TransientOutcome::Pass
        } else {
            TransientOutcome::Fail
        },
        mode: manifest.mode,
        probed: probes.probed(),
        samplerate: manifest.samplerate,
        cycles: cycles.len(),
        passed,
        input_drive_events: cycles.iter().map(|c| c.check.input_drive.len()).sum(),
        output_drive_events: cycles.iter().map(|c| c.check.output_drive.len()).sum(),
        ramp_skew_events: cycles.iter().map(|c| c.check.ramp_skew.len()).sum(),
        ramp_us: RAMP_US,
        skew_us: SKEW_US,
        brake_at_s,
        limit: LIMIT,
        not_probed: probes.inputs.is_none().then_some(OUTPUTS_ONLY),
    };
    Ok(TransientsReport {
        summary,
        per_cycle: cycles,
    })
}

#[cfg(test)]
mod tests;

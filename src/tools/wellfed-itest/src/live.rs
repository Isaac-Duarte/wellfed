//! The live [`Rig`]: the bench's real hotspot, UART captures, device API
//! sessions as the owner's key (requests, and the updates it pushes on a thread
//! of its own) and, with `--power`, the feeder supply relay.
//!
//! The owner's standing authorization (AGENTS.md) covers bringing the
//! hotspot up, provided the prior network state comes back afterwards. So the
//! rig records the hotspot's and the supply's state before touching either,
//! and restores everything in [`LiveRig::restore`], which `Drop` also runs.
//!
//! The UART is held by back-to-back receive-only `wellfed-flash read
//! observe` segments of [`SEGMENT`]. The capture writer flushes each
//! received chunk, so the growing `serial-rx.bin` is read while the segment
//! runs. The chain runs on its own thread ([`Capture`]), so the port stays
//! held while the suite blocks on a build.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::args::ItestArgs;
use wellfed_bench_formats::CAPTURES;
use wellfed_bench_formats::banks::{Bank, Content, Source};
use wellfed_bench_formats::progress::StepState;
use wellfed_formats::require;
use wellfed_relay::{POWER, STRAP, Switch};
use wellfed_rig::commands::Bench;
use wellfed_rig::commands::ota::{self, OtaArgs, Staged, Target, Timing};
use wellfed_rig::device::{Answer, Ask, Device, Intruder, Rebuffed};
use wellfed_rig::hotspot::{self, HotspotError, HotspotFile};
use wellfed_rig::say;
use wellfed_rig::tools::{Capture, CaptureFailure, InstallJob, Installing, PushError};

use super::{Armed, Fault, Hotspot, Result, Rig};

/// Why the live rig could not set the bench up or keep it running.
#[derive(Debug, thiserror::Error)]
pub enum LiveError {
    #[error("{} has no connection", hotspot.display())]
    NoConnection { hotspot: PathBuf },
    #[error("UART port {port} is missing")]
    UartMissing { port: String },
    #[error("nmcli: {0}")]
    Nmcli(std::io::Error),
    #[error(transparent)]
    Hotspot(#[from] HotspotError),
    #[error("nmcli connection {verb} failed: {stderr}")]
    NmcliVerb { verb: &'static str, stderr: String },
    #[error("A power cycle needs --power")]
    NoPower,
    #[error("The UART capture is not running")]
    NoCapture,
    #[error("The UART capture stopped: {0}")]
    CaptureStopped(Arc<CaptureFailure>),
}

/// The observer's own limit for one capture.
const SEGMENT: Duration = Duration::from_secs(180);

pub(super) struct LiveRig<'b> {
    bench: &'b mut Bench,
    name: String,
    started: Instant,
    device: Device,
    /// The owner key file and server config every push uses.
    keys: PathBuf,
    config: PathBuf,
    /// The update being pushed, if any.
    pushing: Option<Box<dyn Installing>>,
    connection: String,
    /// The hotspot's state before the run; `None` until recorded.
    hotspot_was: Option<Hotspot>,
    hotspot_now: Hotspot,
    power: bool,
    power_was_on: Option<bool>,
    /// `On` while an unattended run holds the motor: teardown cuts relay 1.
    kill_armed: Armed,
    capture: Option<Box<dyn Capture>>,
    /// The segment being read, by index, and how far into it.
    reading: (usize, u64),
    /// Root-relative capture directories, for the summary.
    pub(super) captures: Vec<String>,
    restored: bool,
}

impl<'b> LiveRig<'b> {
    /// Check everything that can refuse before the bench changes: configs,
    /// the UART port and unused evidence names.
    pub(super) fn open(bench: &'b mut Bench, args: &ItestArgs) -> Result<Self> {
        let root = bench.root.clone();
        let name = args.name().to_string();
        let hotspot: HotspotFile = wellfed_rig::load(&root, &args.hotspot)?;
        require(!hotspot.connection.is_empty(), || LiveError::NoConnection {
            hotspot: args.hotspot.clone(),
        })?;

        let rig = Self {
            name,
            started: Instant::now(),
            device: Device::open(&root, &args.keys, &args.config)?,
            keys: args.keys.clone(),
            config: args.config.clone(),
            pushing: None,
            connection: hotspot.connection,
            hotspot_was: None,
            hotspot_now: Hotspot::Down,
            power: args.power,
            power_was_on: None,
            kill_armed: Armed::Off,
            capture: None,
            reading: (0, 0),
            captures: Vec::new(),
            restored: false,
            bench,
        };

        require(Path::new(&rig.bench.config.uart_port).exists(), || {
            LiveError::UartMissing {
                port: rig.bench.config.uart_port.clone(),
            }
        })?;
        rig.bench.fresh(&rig.segment_path(1))?;
        Ok(rig)
    }

    /// Bring the bench to its test state: hotspot up, feeder powered (with
    /// `--power`) and the UART held.
    pub(super) fn start(&mut self) -> Result<()> {
        let was = if self.hotspot_active()? {
            Hotspot::Up
        } else {
            Hotspot::Down
        };
        self.hotspot_was = Some(was);
        match was {
            Hotspot::Up => self.hotspot_now = Hotspot::Up,
            Hotspot::Down => self.hotspot(Hotspot::Up)?,
        }

        if self.power {
            let on = self.bench.get(POWER)?;
            self.power_was_on = Some(on);
            if !on {
                self.note("Feeder supply was off; switching it on");
                self.bench.set(STRAP, Switch::Off)?;
            }
            // Also connects a powered feeder's UART, which may be isolated.
            self.bench.power_on()?;
        }

        let (root, name) = (self.bench.root.clone(), self.name.clone());
        let segment = move |n: u32| root.join(CAPTURES).join(format!("{name}-uart-{n:02}"));
        let port = self.bench.config.uart_port.clone();
        self.capture = Some(
            self.bench
                .tools
                .capture(&port, SEGMENT, Box::new(segment))?,
        );
        self.uart()?;
        Ok(())
    }

    fn segment_path(&self, n: u32) -> PathBuf {
        self.bench
            .root
            .join(CAPTURES)
            .join(format!("{}-uart-{n:02}", self.name))
    }

    fn hotspot_active(&self) -> Result<bool> {
        Ok(hotspot::active(&self.connection).map_err(LiveError::from)?)
    }

    /// End the push still running, if any, and log what it said.
    fn stop_push(&mut self) {
        if let Some(mut pushing) = self.pushing.take() {
            pushing.stop();
            for note in pushing.notes() {
                self.note(&format!("  push: {note}"));
            }
        }
    }

    /// Put the bench back as [`Self::start`] found it. Safe to repeat.
    pub(super) fn restore(&mut self) {
        if self.restored {
            return;
        }
        self.restored = true;

        // An unattended run that did not end cleanly: power off first.
        if self.kill_armed == Armed::On
            && let Err(error) = self.bench.cut_power()
        {
            eprintln!("Could not cut relay 1 (cut the feeder supply by hand): {error}");
        }

        self.stop_push();
        if let Some(mut capture) = self.capture.take() {
            capture.stop();
        }
        self.bench.status.capture(None, 0.0);
        if let Some(was) = self.hotspot_was
            && was != self.hotspot_now
            && let Err(error) = self.hotspot(was)
        {
            eprintln!("Could not restore the hotspot {}: {error}", self.connection);
        }
        if self.power_was_on == Some(false)
            && let Err(error) = self.bench.cut_power()
        {
            eprintln!("Could not switch the feeder supply back off: {error}");
        }
    }
}

impl Drop for LiveRig<'_> {
    fn drop(&mut self) {
        self.restore();
    }
}

impl Rig for LiveRig<'_> {
    fn stage(&mut self, args: &OtaArgs) -> Result<Option<Staged>> {
        Ok(self.bench.ota_stage(args, &mut Timing::default())?)
    }

    fn wrote(&mut self, bank: Bank, content: Content) {
        self.bench.wrote(bank, content, Source::Itest);
    }

    fn review_build(&mut self, review: &Path) -> Result<String> {
        let manifest = wellfed_rig::load(&self.bench.root, &review.join("build-manifest.json"))?;
        Ok(ota::build_id(&manifest)?)
    }

    fn offer(&mut self, staged: Option<&Staged>, fault: Fault) -> Result<()> {
        self.stop_push();
        let Some(staged) = staged else {
            return Ok(());
        };
        let job = InstallJob {
            keys: self.keys.clone(),
            config: self.config.clone(),
            manifest: staged.manifest.clone().into(),
            image: staged.image.clone().into(),
            // Stock has no trial to come back from: the owner's statement.
            downgrade: (staged.target == Target::Stock).then(|| ota::KEY_PATH.into()),
            fault: fault.device(),
        };
        if let Some(stage) = fault.stage() {
            self.note(&format!(
                "  Pushing version {} with a fault bank 0 must stop at {stage}",
                staged.version
            ));
        }
        self.pushing = Some(self.bench.tools.install(&job)?);
        Ok(())
    }

    fn offered(&mut self) -> Option<std::result::Result<(), Arc<PushError>>> {
        let pushing = self.pushing.as_mut()?;
        let notes = pushing.notes();
        let finished = pushing.finished();
        for note in notes {
            self.note(&format!("  push: {note}"));
        }
        finished
    }

    /// Through the bench's own link, which the bench server lends its one
    /// session as; the held and intruding sessions below stay this rig's.
    fn device(&mut self, ask: &Ask) -> Result<Answer> {
        let link = self.bench.tools.device(&self.keys, &self.config)?;
        Ok(link.ask(ask)?)
    }

    fn hold(&mut self) -> Result<()> {
        Ok(self.device.hold()?)
    }

    fn held(&mut self, ask: &Ask) -> Result<Answer> {
        Ok(self.device.ask_held(ask)?)
    }

    fn release(&mut self) {
        self.device.release();
    }

    fn intrude(&mut self, intruder: Intruder) -> Result<Rebuffed> {
        Ok(self.device.intrude(intruder)?)
    }

    fn unix_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }

    fn hotspot(&mut self, hotspot: Hotspot) -> Result<()> {
        let verb = hotspot.verb();
        self.note(&format!("  nmcli connection {verb} {}", self.connection));
        let output = Command::new("nmcli")
            .args(["connection", verb, &self.connection])
            .stdin(Stdio::null())
            .output()
            .map_err(LiveError::Nmcli)?;
        require(output.status.success(), || LiveError::NmcliVerb {
            verb,
            stderr: String::from_utf8_lossy(&output.stderr).trim().into(),
        })?;
        self.hotspot_now = hotspot;
        Ok(())
    }

    fn power_cycle(&mut self) -> Result<()> {
        require(self.power, || LiveError::NoPower)?;
        self.note("  Cold boot: supply off, then on");
        self.bench.cut_power()?;
        self.bench.pause(self.bench.off_time)?;
        Ok(self.bench.power_on()?)
    }

    fn kill(&mut self) -> Result<()> {
        require(self.power, || LiveError::NoPower)?;
        self.note("  Relay 1 off: the feeder stays unpowered");
        // `restore` only ever switches the supply off, so it stays off.
        Ok(self.bench.cut_power()?)
    }

    fn arm_kill(&mut self, armed: Armed) {
        self.kill_armed = armed;
    }

    fn uart(&mut self) -> Result<Vec<u8>> {
        let capture = self.capture.as_mut().ok_or(LiveError::NoCapture)?;
        let segments = capture.segments();
        if let Some(failure) = capture.failure() {
            return Err(LiveError::CaptureStopped(failure).into());
        }
        for segment in segments.iter().skip(self.captures.len()) {
            self.captures.push(self.bench.rel(segment));
            self.bench
                .status
                .capture(Some(segment), SEGMENT.as_secs_f64());
        }

        let mut bytes = Vec::new();
        while let Some(segment) = segments.get(self.reading.0) {
            let (index, offset) = &mut self.reading;
            if let Ok(mut file) = File::open(segment.join("serial-rx.bin")) {
                file.seek(SeekFrom::Start(*offset))?;
                *offset += file.read_to_end(&mut bytes)? as u64;
            }
            // The next segment starts only once this one's observe exited,
            // after its last flush, so nothing more arrives here.
            if *index + 1 >= segments.len() {
                break;
            }
            *index += 1;
            *offset = 0;
        }
        Ok(bytes)
    }

    fn wait(&mut self, span: Duration) -> Result<()> {
        Ok(self.bench.pause(span)?)
    }

    fn clock(&self) -> Duration {
        self.started.elapsed()
    }

    fn note(&mut self, text: &str) {
        say!("{text}");
        self.bench.status.line(text);
    }

    fn scenario(&mut self, name: &str, state: StepState) {
        self.bench.status.mark(name, state);
    }
}

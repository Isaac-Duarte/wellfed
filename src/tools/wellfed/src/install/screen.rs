//! `wellfed install` as a person at a terminal reads it: the five steps the
//! Android app numbers, one line per milestone, and one line drawn in place for
//! whatever is counting (a download, a promotion, the code's window).
//!
//! The bench server's job never comes here: with `--prompts lines` the run
//! prints every event as [`crate::report::Printer`] always has, since a job's
//! output is its evidence. `--verbose` shows every event in those same words.
//!
//! Standard error may not be a terminal, or the stock server's log may share
//! it: then nothing is drawn in place, and what counts prints as a line each
//! quarter of the way.

use std::io::Write;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use tokio::task::AbortHandle;
use tokio::time::Instant;
use wellfed_install::Report;
use wellfed_install::event::{Event, Wait};

use crate::report::line;

/// How a run's progress may be drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Style {
    /// A terminal of its own: lines drawn in place, the code highlighted.
    Live,
    /// A file, a pipe, or a terminal the stock server's log writes to too.
    Plain,
}

/// Which events are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Detail {
    Milestones,
    /// Every event, in the words `--prompts lines` prints.
    Everything,
}

/// The install's steps, named as the app names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Step {
    Reset = 1,
    Reach = 2,
    Yours = 3,
    Claim = 4,
    Permanent = 5,
}

impl Step {
    const fn title(self) -> &'static str {
        match self {
            Self::Reset => "Reset the feeder",
            Self::Reach => "Tell the feeder how to reach this computer",
            Self::Yours => "A feeder connected. Is it yours?",
            Self::Claim => "Wellfed is running. Now claim it.",
            Self::Permanent => "Making Wellfed permanent",
        }
    }
}

/// What counts up, drawn in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Meter {
    Percent {
        label: &'static str,
        percent: u8,
    },
    /// Bytes of stock's bank read: its length is the bank's, known only roughly here.
    Read(usize),
}

/// How an event shows.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shown {
    Hidden,
    Line(String),
    Meter(Meter),
}

const WIDTH: usize = 24;
const CLEAR_LINE: &str = "\r\x1b[2K";
const BELL: &str = "\x07";

/// The step `event` belongs to, if it starts one.
const fn step_of(event: &Event) -> Option<Step> {
    match event {
        Event::DevicePinned { .. } => Some(Step::Yours),
        Event::SetupStarting
        | Event::KeyKnown
        | Event::KeyKnownAt { .. }
        | Event::Rejoining { .. } => Some(Step::Claim),
        Event::ReplacementAnswers { .. } => Some(Step::Permanent),
        _ => None,
    }
}

/// What a person at a terminal reads for `event`. What only says the run is
/// still at work hides; a word the bench's evidence keeps stays in
/// [`crate::report::line`], which most events here simply use.
fn shown(event: &Event) -> Shown {
    match event {
        Event::StockServerStarted
        | Event::FeederTried { .. }
        | Event::TrialClosed
        | Event::Floor(_)
        | Event::Signed { .. } => Shown::Hidden,
        Event::Waiting(Wait::NoFeeder) => {
            Shown::Line("Waiting for the feeder to connect to this computer.".to_string())
        }
        Event::Requesting { .. } => Shown::Line(
            "Installing Wellfed: stock was asked once to download it from this computer.\n\
             In a few minutes the feeder will probably want a code: you get 60 s to key it in on \
             its panel, so stay near it."
                .to_string(),
        ),
        Event::StockProgress(percent) => Shown::Meter(Meter::Percent {
            label: "Stock downloading Wellfed",
            percent: (*percent).min(100),
        }),
        Event::SetupStarting => Shown::Line(format!(
            "{}\nGet to the feeder now: the code shows here as soon as it is ready, and you get \
             60 s to key it in.",
            line(event)
        )),
        Event::ReadingBank1 { .. } => Shown::Line(
            "Copying stock from the feeder to this computer, before anything overwrites it."
                .to_string(),
        ),
        Event::Bank1Read(read) => Shown::Meter(Meter::Read(*read)),
        Event::Promoting => Shown::Line(
            "Writing Wellfed over the feeder's copy of stock. This computer holds the checked \
             copy."
                .to_string(),
        ),
        Event::Written { written, length } => Shown::Meter(Meter::Percent {
            label: "Writing Wellfed into bank 1",
            percent: percent(*written, *length),
        }),
        Event::Installed { .. } => {
            Shown::Line(format!("{}\nTo see it, run: wellfed status", line(event)))
        }
        other => Shown::Line(line(other)),
    }
}

fn percent(done: u32, of: u32) -> u8 {
    let share = u64::from(done) * 100 / u64::from(of.max(1));
    u8::try_from(share.min(100)).unwrap_or(100)
}

impl Meter {
    fn text(self) -> String {
        match self {
            Self::Percent { label, percent } => {
                let filled = WIDTH * usize::from(percent) / 100;
                format!(
                    "  {label}: [{}{}] {percent}%",
                    "#".repeat(filled),
                    "-".repeat(WIDTH - filled)
                )
            }
            Self::Read(read) => format!(
                "  Copying stock: {:.1} MB of about 1.4 MB",
                // Shown to a tenth: the precision a float loses here is never seen.
                f64::from(u32::try_from(read / 1_000).unwrap_or(u32::MAX)) / 1_000.0
            ),
        }
    }
}

/// What `seconds` left of a window reads as.
fn countdown_text(label: &str, seconds: u64) -> String {
    if seconds == 0 {
        format!("  {label}: time is up, waiting for the feeder to say so")
    } else {
        format!("  {label}: {seconds} s left")
    }
}

struct State {
    output: Box<dyn Write + Send>,
    style: Style,
    detail: Detail,
    step: Option<Step>,
    /// What is drawn in place now.
    live: Option<String>,
    /// The quarter each counting label last printed, drawn plain.
    quarter: Option<(&'static str, u8)>,
    ticker: Option<AbortHandle>,
    /// Names the countdown that may draw: one the screen has moved past may
    /// still be waiting for the lock.
    countdown: u64,
}

impl State {
    fn write(&mut self, text: &str) {
        // A closed standard error must not stop an install; it has no other way
        // to say so either.
        let _ = self.output.write_all(text.as_bytes());
        let _ = self.output.flush();
    }

    fn stop_counting(&mut self) {
        self.countdown += 1;
        if let Some(ticker) = self.ticker.take() {
            ticker.abort();
        }
    }

    /// Wipe the line drawn in place, for what prints next.
    fn clear_live(&mut self) {
        self.stop_counting();
        if self.live.take().is_some() {
            self.write(CLEAR_LINE);
        }
    }

    fn line(&mut self, text: &str) {
        self.clear_live();
        self.write(&format!("{text}\n"));
    }

    fn draw(&mut self, text: String) {
        self.write(&format!("{CLEAR_LINE}{text}"));
        self.live = Some(text);
    }

    fn enter(&mut self, step: Step) {
        if self.step.is_some_and(|at| at >= step) {
            return;
        }
        self.step = Some(step);
        self.line(&format!("\nStep {} of 5: {}", step as u8, step.title()));
    }

    fn meter(&mut self, meter: Meter) {
        match self.style {
            Style::Live => {
                self.stop_counting();
                self.draw(meter.text());
            }
            Style::Plain => {
                let Meter::Percent { label, percent } = meter else {
                    return;
                };
                let quarter = percent / 25;
                let said = self.quarter.filter(|(at, _)| *at == label).map(|(_, q)| q);
                if quarter > said.unwrap_or(0) {
                    self.quarter = Some((label, quarter));
                    self.line(&format!("  {label}: {percent}%"));
                }
            }
        }
    }
}

/// The run's one writer on standard error, shared by the person's prompts and
/// the run's progress.
#[derive(Clone)]
pub(crate) struct Screen(Arc<Mutex<State>>);

impl Screen {
    pub(crate) fn new(output: Box<dyn Write + Send>, style: Style, detail: Detail) -> Self {
        Self(Arc::new(Mutex::new(State {
            output,
            style,
            detail,
            step: None,
            live: None,
            quarter: None,
            ticker: None,
            countdown: 0,
        })))
    }

    /// The state, even if a thread panicked with it: what it holds is only
    /// what is on the screen.
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn line(&self, text: &str) {
        self.state().line(text);
    }

    pub(crate) fn enter(&self, step: Step) {
        self.state().enter(step);
    }

    /// Clear the way for a prompt: what counts stops being drawn.
    pub(crate) fn pause(&self) {
        self.state().clear_live();
    }

    /// `presses`, as `positions` and names, so they cannot be missed: a blank
    /// line around them, a bell, and the digits highlighted on a terminal.
    pub(crate) fn code(&self, heading: &str, positions: &str, names: &str, footer: &str) {
        let mut state = self.state();
        let (bell, on, off) = match state.style {
            Style::Live => (BELL, "\x1b[1;7m", "\x1b[0m"),
            Style::Plain => ("", "", ""),
        };
        let block = format!(
            "{bell}\n>>> {heading}\n\n        {on} {positions} {off}\n        {names}\n\n{footer}\n"
        );
        state.line(&block);
    }

    /// Count `seconds` down in place until the next thing is shown. Plain, it
    /// shows nothing: the code's own line says how long the window is.
    pub(crate) fn countdown(&self, label: &'static str, seconds: u16) {
        let this = {
            let mut state = self.state();
            if state.style != Style::Live {
                return;
            }
            state.stop_counting();
            state.countdown
        };
        let end = Instant::now() + Duration::from_secs(seconds.into());
        let screen = self.clone();
        let ticker = tokio::spawn(async move {
            loop {
                let left = end.saturating_duration_since(Instant::now());
                // Rounded up: it reads 60 first, and 0 only once time is up.
                let seconds = left.as_secs() + u64::from(left.subsec_nanos() > 0);
                {
                    let mut state = screen.state();
                    if state.countdown != this {
                        return;
                    }
                    state.draw(countdown_text(label, seconds));
                }
                if seconds == 0 {
                    return;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        self.state().ticker = Some(ticker.abort_handle());
    }
}

impl Report for Screen {
    fn event(&self, event: Event) {
        let mut state = self.state();
        if let Some(step) = step_of(&event) {
            state.enter(step);
        }
        if state.detail == Detail::Everything {
            state.line(&line(&event));
            return;
        }
        match shown(&event) {
            Shown::Hidden => {}
            Shown::Line(text) => state.line(&text),
            Shown::Meter(meter) => state.meter(meter),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// What a screen wrote, for a test to read.
    #[derive(Clone, Default)]
    pub(crate) struct Written(Arc<Mutex<Vec<u8>>>);

    impl Written {
        pub(crate) fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
        }
    }

    impl Write for Written {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    pub(crate) fn screen(style: Style, detail: Detail) -> (Screen, Written) {
        let written = Written::default();
        (
            Screen::new(Box::new(written.clone()), style, detail),
            written,
        )
    }

    fn written_all(screen: &Screen) {
        for written in (0..16).map(|n| n * 66_816) {
            screen.event(Event::Written {
                written,
                length: 1_069_056,
            });
        }
        screen.event(Event::Written {
            written: 1_069_056,
            length: 1_069_056,
        });
    }

    /// What the owner saw as 16 lines of bytes is one line drawn in place.
    #[test]
    fn a_terminal_draws_the_promotion_in_place() {
        let (screen, written) = screen(Style::Live, Detail::Milestones);
        written_all(&screen);
        screen.event(Event::Activated);

        let text = written.text();
        assert_eq!(text.lines().count(), 1, "{text:?}");
        assert!(text.contains("Writing Wellfed into bank 1: ["), "{text:?}");
        assert!(text.contains("100%"), "{text:?}");
        assert!(!text.contains("bytes written"), "{text:?}");
        // The last thing drawn is wiped, and the milestone takes its place.
        assert!(
            text.ends_with("\r\x1b[2KActivated: the feeder restarts into bank 1.\n"),
            "{text:?}"
        );
    }

    /// Not a terminal: plain lines, a quarter at a time, and no escapes.
    #[test]
    fn a_pipe_gets_a_line_each_quarter() {
        let (screen, written) = screen(Style::Plain, Detail::Milestones);
        written_all(&screen);
        for percent in [10, 30, 30, 60, 100] {
            screen.event(Event::StockProgress(percent));
        }
        screen.event(Event::Bank1Read(768));

        let text = written.text();
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                "  Writing Wellfed into bank 1: 25%",
                "  Writing Wellfed into bank 1: 50%",
                "  Writing Wellfed into bank 1: 75%",
                "  Writing Wellfed into bank 1: 100%",
                "  Stock downloading Wellfed: 30%",
                "  Stock downloading Wellfed: 60%",
                "  Stock downloading Wellfed: 100%",
            ]
        );
        assert!(!text.contains('\x1b') && !text.contains('\r'), "{text:?}");
    }

    /// Steps go forward once each, with the app's names, and what only says
    /// the run is at work stays out of the way.
    #[test]
    fn milestones_are_numbered_and_noise_is_hidden() {
        let (screen, written) = screen(Style::Plain, Detail::Milestones);
        let identity = [0xab; 32];
        screen.enter(Step::Reset);
        screen.enter(Step::Reach);
        screen.event(Event::StockServerStarted);
        screen.event(Event::FeederTried {
            address: "10.0.0.2:7100".parse().unwrap(),
            answered: false,
        });
        screen.event(Event::Floor(7));
        screen.event(Event::SetupStarting);
        screen.event(Event::KeyKnown);
        screen.event(Event::Rejoining { identity });
        screen.event(Event::ReplacementAnswers {
            build: "ab".repeat(32),
            bank: 0,
        });
        screen.enter(Step::Reach);

        let text = written.text();
        assert!(text.contains("Step 1 of 5: Reset the feeder\n"), "{text}");
        assert!(
            text.contains("Step 2 of 5: Tell the feeder how to reach this computer\n"),
            "{text}"
        );
        assert!(
            text.contains("Step 4 of 5: Wellfed is running. Now claim it.\n"),
            "{text}"
        );
        assert!(
            text.contains("Step 5 of 5: Making Wellfed permanent\n"),
            "{text}"
        );
        assert_eq!(text.matches("Step 2 of 5").count(), 1, "{text}");
        assert_eq!(text.matches("Step 4 of 5").count(), 1, "{text}");
        assert!(!text.contains("Started the stock server"), "{text}");
        assert!(!text.contains("No answer from the feeder"), "{text}");
        assert!(!text.contains("floor"), "{text}");
        // The third step's words are the app's too, once the feeder connected.
        assert_eq!(Step::Yours.title(), "A feeder connected. Is it yours?");
    }

    /// The code cannot come as a surprise: the run says it is coming when
    /// stock starts its download, and again when it is close.
    #[test]
    fn the_code_is_announced_before_it_can_appear() {
        for event in [
            Event::Requesting {
                build: "ab".repeat(32),
                request: uuid::Uuid::from_u128(7),
            },
            Event::SetupStarting,
        ] {
            let Shown::Line(text) = shown(&event) else {
                panic!("{event:?} shows");
            };
            assert!(text.contains("60 s"), "{text}");
            assert!(text.contains("key it in"), "{text}");
        }
    }

    /// `--verbose` shows every event in the words the bench's evidence keeps.
    #[test]
    fn verbose_shows_every_event_in_the_plain_words() {
        let (screen, written) = screen(Style::Plain, Detail::Everything);
        screen.event(Event::StockServerStarted);
        screen.event(Event::Written {
            written: 512,
            length: 1024,
        });
        let text = written.text();
        assert!(text.contains("Started the stock server"), "{text}");
        assert!(text.contains("  512 of 1024 bytes written\n"), "{text}");
    }

    #[test]
    fn a_code_stands_out_on_a_terminal_and_stays_plain_elsewhere() {
        let (live, written) = screen(Style::Live, Detail::Milestones);
        live.code("PRESS", "2 1 3", "Dispense, Lock, Mic", "Footer");
        let text = written.text();
        assert!(text.starts_with('\x07'), "{text:?}");
        assert!(text.contains("\x1b[1;7m 2 1 3 \x1b[0m"), "{text:?}");

        let (plain, written) = screen(Style::Plain, Detail::Milestones);
        plain.code("PRESS", "2 1 3", "Dispense, Lock, Mic", "Footer");
        let text = written.text();
        assert!(text.contains(" 2 1 3 "), "{text:?}");
        assert!(!text.contains('\x1b') && !text.contains('\x07'), "{text:?}");
    }

    #[test]
    fn a_countdown_counts_in_seconds_and_ends_plainly() {
        assert_eq!(countdown_text("Code", 42), "  Code: 42 s left");
        assert!(countdown_text("Code", 0).contains("time is up"));
    }

    /// The window counts down in place, and anything shown next ends it, so a
    /// tick left behind never draws over what follows.
    #[tokio::test]
    async fn a_countdown_runs_until_something_else_shows() {
        let (screen, written) = screen(Style::Live, Detail::Milestones);
        screen.countdown("Key in the code", 60);
        // The first tick runs as soon as the task is scheduled.
        while !written.text().contains("s left") {
            tokio::task::yield_now().await;
        }
        screen.line("Paired.");
        let at_end = written.text();
        assert!(at_end.ends_with("Paired.\n"), "{at_end:?}");

        tokio::task::yield_now().await;
        assert_eq!(written.text(), at_end);

        // Plain, nothing is drawn or started.
        let (plain, written) = super::tests::screen(Style::Plain, Detail::Milestones);
        plain.countdown("Key in the code", 60);
        assert_eq!(written.text(), "");
    }
}

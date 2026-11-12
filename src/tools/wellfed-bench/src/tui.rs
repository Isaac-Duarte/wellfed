//! `serve`'s full-screen console, drawn on `/dev/tty` from the server's
//! live state, with the console's lines (its own and every job's output)
//! in a scrolling log.
//!
//! It runs on a thread of its own and reads only the state and a channel,
//! so a slow terminal never holds the output pump: the pump drops console
//! lines rather than wait, and the job's stream keeps them. While the
//! flash client writes, it redraws at most twice a second, so the bench
//! host spends no more than that on it during a write. `q` and Ctrl+C
//! (which raw mode delivers as a key, not a signal) ask for the same stop
//! as SIGINT: cancel the job, wait for its cleanup, then release the bench.
//! A job's question shows above the log, read-only: the `answer` command
//! and an agent answer it.
//! While the flash client holds the UART, a press only shows the
//! confirmation in the footer, and `y` within 5 s stops. The console sends
//! each press, and the server decides it against the UART as it is then.
//! Keys pasted in, and every key after the first `q` of one read, press
//! nothing.
//!
//! Should the console end on its own (a terminal error, its terminal hanging
//! up, a panic), it tells the server, which puts its lines back on the
//! terminal and keeps serving. A hangup also reaches the server as SIGHUP,
//! which `stopping` holds back during a write; the console's end hurries
//! nothing, and leaves nothing for [`Tui::finish`] to wait on.
//!
//! `console` draws the same screen from another process ([`attached`]): the
//! raw mode, the scrolling, the pacing and the drawing here are shared, and
//! a [`Feed`] says where the state and the lines come from and what `q` does.

pub(crate) mod attached;
mod keys;
mod view;

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, IsTerminal, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc::UnboundedSender;
use wellfed_bench::server::Store;
use wellfed_bench::server::stopping::{Ask, CONFIRM_PROMPT, CONFIRM_WINDOW, Held};
use wellfed_bench_formats::live::{BenchState, Uart};

use keys::{Input, Key, Keys};
use view::Screen;

/// The console lines kept for scrolling.
const KEPT: usize = 5_000;
/// Between frames while nothing writes flash.
const FRAME: Duration = Duration::from_millis(250);
/// Between frames during a write: about 2 Hz.
const WRITING_FRAME: Duration = Duration::from_millis(500);
/// The log lines printed back on the terminal after the screen closes.
const TAIL_AFTER: usize = 30;
/// The footer's keys on the server's own console.
const SERVE_KEYS: &str =
    "q or Ctrl+C stops the server (cancel, cleanup, release) · PgUp/PgDn scroll · End follows";

/// Whether the full-screen console can run: a terminal on stdout and a
/// controlling terminal to draw on, unless `--plain` asked for lines.
pub(crate) fn available(plain: bool) -> bool {
    !plain && io::stdout().is_terminal() && tty().is_ok()
}

/// The controlling terminal. Not `/dev/tty` on macOS, whose `poll` refuses
/// that device (`POLLNVAL`, which [`keys`] would read as a hangup): there the
/// terminal's own device, as stdin, stdout or stderr names it.
#[cfg(not(target_os = "macos"))]
fn terminal_path() -> std::io::Result<std::path::PathBuf> {
    Ok("/dev/tty".into())
}

#[cfg(target_os = "macos")]
fn terminal_path() -> std::io::Result<std::path::PathBuf> {
    use std::os::unix::ffi::OsStringExt;

    let name = rustix::termios::ttyname(std::io::stdin(), Vec::new())
        .or_else(|_| rustix::termios::ttyname(std::io::stdout(), Vec::new()))
        .or_else(|_| rustix::termios::ttyname(std::io::stderr(), Vec::new()))?;
    Ok(std::ffi::OsString::from_vec(name.into_bytes()).into())
}

fn tty() -> io::Result<File> {
    File::options()
        .read(true)
        .write(true)
        .open(terminal_path()?)
}

/// The running console. Dropped without [`Self::finish`], as when the
/// main thread panics, it still closes the screen and gives the terminal
/// back out of raw mode.
pub(crate) struct Tui {
    done: Arc<AtomicBool>,
    thread: Option<JoinHandle<io::Result<()>>>,
}

impl Tui {
    /// Take over the terminal and draw `store` until [`Self::finish`];
    /// `q` or Ctrl+C sends [`Ask::Quit`] on `asks`, `y` [`Ask::ConfirmQuit`],
    /// and a console that ends on its own [`Ask::ConsoleEnded`].
    pub(crate) fn start(
        store: Store,
        lines: Receiver<String>,
        asks: UnboundedSender<Ask>,
    ) -> io::Result<Self> {
        let tty = tty()?;
        let done = Arc::new(AtomicBool::new(false));
        let finished = Arc::clone(&done);
        let thread = std::thread::Builder::new()
            .name("console".into())
            .spawn(move || {
                // Dropped last, after the screen is closed, a panic included.
                let _ended = Ended {
                    asks: asks.clone(),
                    done: Arc::clone(&finished),
                };
                run(
                    tty,
                    &mut Served {
                        store: &store,
                        lines: &lines,
                        asks: &asks,
                        done: &finished,
                        quitting: Quitting::default(),
                    },
                )
            })?;
        Ok(Self {
            done,
            thread: Some(thread),
        })
    }

    /// Close the screen, after the server has stopped, and give the
    /// terminal back with the log's last lines on it.
    pub(crate) fn finish(mut self) -> io::Result<()> {
        self.close()
    }

    fn close(&mut self) -> io::Result<()> {
        self.done.store(true, Ordering::SeqCst);
        match self.thread.take() {
            Some(thread) => thread
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("the console thread panicked"))),
            None => Ok(()),
        }
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

/// Tells the server when the console thread ends without being asked to.
struct Ended {
    asks: UnboundedSender<Ask>,
    done: Arc<AtomicBool>,
}

impl Drop for Ended {
    fn drop(&mut self) {
        if !self.done.load(Ordering::SeqCst) {
            let _ = self.asks.send(Ask::ConsoleEnded);
        }
    }
}

/// The confirmation the footer shows after a press during a write. Only
/// shown: the server decides each press itself.
#[derive(Default)]
struct Quitting {
    asked: Option<Instant>,
}

impl Quitting {
    /// A press at `at`, with the UART as the state last had it.
    fn press(&mut self, uart: &Uart, at: Instant) {
        self.asked = (Held::of(uart) != Held::Free).then_some(at);
    }

    /// `y` was pressed: the question is answered either way.
    fn confirmed(&mut self) {
        self.asked = None;
    }

    /// The footer's confirmation, while a press waits for its `y`.
    fn notice(&self, at: Instant) -> Option<&'static str> {
        self.asked
            .is_some_and(|asked| at.saturating_duration_since(asked) <= CONFIRM_WINDOW)
            .then_some(CONFIRM_PROMPT)
    }
}

/// Raw mode and the alternate screen, left on every way out, a panic
/// included.
struct Screenful {
    tty: File,
}

impl Screenful {
    fn enter(mut tty: File) -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        // Bracketed paste, so a paste's keys arrive marked and are dropped.
        crossterm::execute!(
            tty,
            EnterAlternateScreen,
            crossterm::cursor::Hide,
            crossterm::event::EnableBracketedPaste
        )?;
        Ok(Self { tty })
    }
}

impl Drop for Screenful {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            self.tty,
            crossterm::event::DisableBracketedPaste,
            LeaveAlternateScreen,
            crossterm::cursor::Show
        );
        let _ = terminal::disable_raw_mode();
    }
}

/// End on a terminal that has hung up. `Terminal`'s drop would try to show the
/// cursor on it and `eprintln!` the failure: into the running job's log under
/// `serve`, and on a dead stderr, which panics, under `console`. It happens
/// once in a process, so what the screen holds is simply left behind.
fn abandon(terminal: Terminal<CrosstermBackend<File>>) -> io::Result<()> {
    std::mem::forget(terminal);
    Ok(())
}

/// Where a screen's state and lines come from, and what its keys do.
trait Feed {
    /// The footer's keys.
    fn keys(&self) -> &'static str;
    /// The state as of now.
    fn state(&self) -> BenchState;
    /// The next line waiting for the log, without waiting for one.
    fn line(&mut self) -> Option<String>;
    /// What the footer shows instead of the keys.
    fn notice(&self, at: Instant) -> Option<&'static str>;
    /// `q` or Ctrl+C was pressed.
    fn quit_pressed(&mut self, at: Instant);
    /// `y` was pressed.
    fn confirm_pressed(&mut self);
    /// Whether to close the screen.
    fn finished(&self) -> bool;
}

/// The server's own console: its store and console lines, and `q` asks it
/// to stop.
struct Served<'a> {
    store: &'a Store,
    lines: &'a Receiver<String>,
    asks: &'a UnboundedSender<Ask>,
    done: &'a AtomicBool,
    quitting: Quitting,
}

impl Feed for Served<'_> {
    fn keys(&self) -> &'static str {
        SERVE_KEYS
    }

    fn state(&self) -> BenchState {
        self.store.read()
    }

    fn line(&mut self) -> Option<String> {
        self.lines.try_recv().ok()
    }

    fn notice(&self, at: Instant) -> Option<&'static str> {
        self.quitting.notice(at)
    }

    fn quit_pressed(&mut self, at: Instant) {
        let uart = self.store.read().uart;
        self.quitting.press(&uart, at);
        let _ = self.asks.send(Ask::Quit);
    }

    fn confirm_pressed(&mut self) {
        self.quitting.confirmed();
        let _ = self.asks.send(Ask::ConfirmQuit);
    }

    fn finished(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }
}

fn run(tty: File, feed: &mut impl Feed) -> io::Result<()> {
    let mut after = tty.try_clone()?;
    let mut keys = Keys::new(tty.try_clone()?);
    let screen = Screenful::enter(tty.try_clone()?)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(tty))?;
    let mut log = VecDeque::new();
    let mut scroll = 0;
    let mut drawn: Option<Instant> = None;
    let mut noticed = false;

    loop {
        while let Some(line) = feed.line() {
            push(&mut log, line);
            // Scrolled up, the view holds still as lines arrive.
            if scroll > 0 {
                scroll = (scroll + 1).min(log.len());
            }
        }
        let state = feed.state();
        let frame = if matches!(state.uart, Uart::Writing { .. }) {
            WRITING_FRAME
        } else {
            FRAME
        };
        let notice = feed.notice(Instant::now());
        // The confirmation shows, and goes, at once.
        if notice.is_some() != noticed {
            drawn = None;
            noticed = notice.is_some();
        }
        if drawn.is_none_or(|at| at.elapsed() >= frame) {
            let draw = terminal
                .draw(|frame| {
                    view::draw(
                        frame,
                        &Screen {
                            state: &state,
                            log: &log,
                            scroll,
                            notice,
                            keys: feed.keys(),
                        },
                    );
                })
                .map(|_| ());
            // A hangup between frames fails the draw before the wait below
            // would see it, and is no more an error here than there.
            match draw {
                Ok(()) => {}
                Err(_) if keys.hung_up() => return abandon(terminal),
                Err(error) => return Err(error),
            }
            drawn = Some(Instant::now());
        }
        if feed.finished() {
            break;
        }

        let wait = drawn.map_or(frame, |at| frame.saturating_sub(at.elapsed()));
        let pressed = match keys.wait(wait)? {
            Input::Keys(pressed) => pressed,
            Input::Nothing => continue,
            // Nothing can be drawn or printed any more: no tail either.
            Input::HungUp => return abandon(terminal),
        };

        let page = usize::from(terminal.size()?.height / 2).max(1);
        // One read is one press at most: two `q`s in it (a held key) are
        // one, and a `y` in the read that asked is no answer to it.
        let quit = pressed.contains(&Key::Quit);
        if quit {
            feed.quit_pressed(Instant::now());
            drawn = None;
        }
        for key in pressed {
            match key {
                Key::Quit => {}
                Key::Confirm if quit => {}
                Key::Confirm => {
                    feed.confirm_pressed();
                    drawn = None;
                }
                Key::Up => scroll = (scroll + 1).min(log.len()),
                Key::PageUp => scroll = (scroll + page).min(log.len()),
                Key::Down => scroll = scroll.saturating_sub(1),
                Key::PageDown => scroll = scroll.saturating_sub(page),
                Key::End => scroll = 0,
                Key::Home => scroll = log.len(),
            }
        }
    }

    drop(terminal);
    drop(screen);
    // Off the screen, which drops control characters itself, the lines
    // go to the terminal as plain text.
    let tail = log.len().saturating_sub(TAIL_AFTER);
    for line in log.range(tail..) {
        writeln!(after, "{}", wellfed_bench::server::printable(line))?;
    }
    Ok(())
}

fn push(log: &mut VecDeque<String>, line: String) {
    if log.len() == KEPT {
        log.pop_front();
    }
    log.push_back(line);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const WRITING: Uart = Uart::Writing {
        done: 3,
        total: 19,
        counted: true,
    };

    #[test]
    fn q_during_a_write_shows_the_confirmation_until_y_or_five_seconds() {
        let start = Instant::now();
        let mut quitting = Quitting::default();

        quitting.press(&WRITING, start);
        assert_eq!(quitting.notice(start), Some(CONFIRM_PROMPT));
        quitting.confirmed();
        assert_eq!(quitting.notice(start), None);

        // In the loader too, for 5 s.
        quitting.press(&Uart::Loader, start);
        assert_eq!(
            quitting.notice(start + Duration::from_secs(4)),
            Some(CONFIRM_PROMPT)
        );
        assert_eq!(quitting.notice(start + Duration::from_secs(6)), None);

        // With the UART free, nothing to confirm.
        let mut idle = Quitting::default();
        idle.press(&Uart::Idle, start);
        assert_eq!(idle.notice(start), None);
    }

    #[test]
    fn a_console_that_ends_unasked_tells_the_server_and_a_finished_one_does_not() {
        let (asks, mut asked) = tokio::sync::mpsc::unbounded_channel();
        let done = Arc::new(AtomicBool::new(false));

        drop(Ended {
            asks: asks.clone(),
            done: Arc::clone(&done),
        });
        assert_eq!(asked.try_recv().ok(), Some(Ask::ConsoleEnded));

        done.store(true, Ordering::SeqCst);
        drop(Ended { asks, done });
        assert!(asked.try_recv().is_err());
    }
}

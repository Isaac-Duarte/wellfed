//! The `restore-stock` job, which runs one tool to its end rather than a rig
//! sequence: `wellfed restore-stock --prompts lines`, as a child of the server,
//! so the server never stops for it. Nothing here retries: a press is typed
//! once, and the offer is the tool's own, which never goes again once its push
//! returned.
//!
//! The tool asks the person at the feeder for one thing, the Dispense press
//! that enrolls a fresh update key, as a [`Prompt`] line on its standard output,
//! and it reads one [`Answer`] line back on its standard input. The question
//! goes into the live state as a [`PendingPrompt`], which the console shows and
//! the CLI and an agent answer, until the server takes an answer for it
//! (`Shared::answer`) or the job ends. Its answers carry no number and are read
//! in order, so an answer reaches it only for the question it is waiting on,
//! never ahead of one.
//!
//! Its standard error is the job's output. A standard output line that is no
//! prompt means the tool and the server disagree about what is asked, and
//! nothing it waits for could be answered safely: the job shows the line and
//! stops the tool.
//!
//! With `--virtual-panel` the job answers the press itself: it types Dispense
//! to a running `virtual-panel` build ([`Bench::press`]), from this thread, as
//! a `wellfed-flash read press` child under the server's lease. The server's
//! UART capture is already stopped for every job and starts again only once it
//! ends, so the port is free for it. A press that does not echo and release
//! stops the tool, and nothing is pressed again: a Dispense pressed twice may
//! feed.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use clap::Args;
use wellfed_bench_formats::banks::{Bank, Content, Source};
use wellfed_bench_formats::live::{ActivityKind, AnswerError, JobId, PendingPrompt};
use wellfed_core::buttons::{Button, Buttons};
use wellfed_core::virtual_panel::DEFAULT_HOLD_MS;
use wellfed_install::prompt::{Answer, DispensePurpose, Prompt};
use wellfed_rig::commands::Bench;
use wellfed_rig::tools::own_group_with_input;

use crate::server::jobs::{JobError, ToolPaths, press_name};
use crate::server::state::Store;

/// How often the job looks at its cancel while its child is quiet.
const TICK: Duration = Duration::from_millis(100);
/// How long the job waits for the server's session to show it closed for the
/// job: a session still opening sees the hold once its attempt ends.
const PAUSE_WAIT: Duration = Duration::from_secs(60);

/// `restore-stock`: the flags of `wellfed restore-stock` the bench uses. The
/// job adds `--prompts lines`.
#[derive(Debug, Clone, Args)]
pub struct RestoreStockArgs {
    /// A stock 1.1.15 image to restore instead of the kept backup; it must
    /// match the pinned image exactly.
    #[arg(long)]
    pub backup: Option<PathBuf>,
    /// An update-signing key the feeder already trusts. Otherwise a fresh
    /// key is made and enrolled with a Dispense press.
    #[arg(long)]
    pub key: Option<PathBuf>,
    /// The version the stock manifest carries; otherwise one above the
    /// feeder's floor.
    #[arg(long)]
    pub version: Option<u32>,
    /// Answer the Dispense press by typing it to a running virtual-panel
    /// build. The server's own flag; the tool never sees it.
    #[arg(long)]
    pub virtual_panel: bool,
    /// Required with --virtual-panel, whose Dispense press can feed: the
    /// hopper is empty. The server's own flag; the tool never sees it.
    #[arg(long)]
    pub confirm_motion: bool,
    /// This computer's client key; the tool's default otherwise.
    #[arg(long)]
    pub keys: Option<PathBuf>,
    /// The feeder's identity, as a hex prefix, when the key file knows more
    /// than one.
    #[arg(long)]
    pub feeder: Option<String>,
    /// The feeder's address; found over mDNS otherwise.
    #[arg(long)]
    pub address: Option<String>,
}

impl RestoreStockArgs {
    /// The tool's arguments, each value one `--flag=value` word.
    fn arguments(&self) -> Vec<OsString> {
        let mut arguments: Vec<OsString> = vec!["restore-stock".into(), "--prompts=lines".into()];
        let mut value = |flag: &str, value: &dyn AsRef<std::ffi::OsStr>| {
            let mut word = OsString::from(format!("--{flag}="));
            word.push(value);
            arguments.push(word);
        };
        if let Some(backup) = &self.backup {
            value("backup", backup);
        }
        if let Some(key) = &self.key {
            value("key", key);
        }
        if let Some(version) = self.version {
            value("version", &version.to_string());
        }
        if let Some(keys) = &self.keys {
            value("keys", keys);
        }
        if let Some(feeder) = &self.feeder {
            value("feeder", feeder);
        }
        if let Some(address) = &self.address {
            value("address", address);
        }
        arguments
    }
}

/// Why a tool job stopped.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ToolJobError {
    #[error(
        "{} not found; build it with cargo build --release -p wellfed or pass serve --wellfed-bin",
        path.display()
    )]
    WellfedMissing { path: PathBuf },
    /// The tool wrote a line on its standard output that is no prompt; it was
    /// stopped.
    #[error("wellfed wrote {line:?} where a prompt goes; it was stopped")]
    Garbled { line: String },
    /// The press for the tool's panel prompt did not echo and release; the
    /// tool was stopped, and nothing was pressed again.
    #[error(
        "the virtual panel's press for the {prompt} prompt ({chord}) did not echo and release; wellfed was stopped and nothing is pressed again"
    )]
    PressFailed {
        prompt: &'static str,
        chord: Buttons,
    },
    /// The tool stopped: its own last words, as it printed them.
    #[error("{}", message.as_deref().unwrap_or("wellfed stopped without saying why"))]
    ToolStopped { code: i32, message: Option<String> },
    /// The tool opens its own sessions only beside a server session shown
    /// paused for its job, and it did not show that in time.
    #[error(
        "The bench server's feeder session did not show itself paused for job {job} within {} s; nothing was started",
        PAUSE_WAIT.as_secs()
    )]
    NotPaused { job: JobId },
}

/// What the job's loop hears: its child's lines, and the answers the
/// server takes for it.
pub(crate) enum Event {
    /// A line for the job's output.
    Output(String),
    Prompt(Prompt),
    /// A standard output line of the tool's that is no prompt.
    Garbled(String),
    /// One of the child's output pipes reached its end.
    Closed,
    Answer(Answer),
}

/// The job's side of its questions: it asks through the live state and
/// hears the answers the server took.
pub(crate) struct Asking {
    job: JobId,
    store: Store,
    sender: mpsc::Sender<Event>,
    events: mpsc::Receiver<Event>,
}

/// The server's side: where an answer it took for the job goes.
#[derive(Clone)]
pub(crate) struct Answering {
    sender: mpsc::Sender<Event>,
}

/// The two ends for job `job`. Every job gets them; only `restore-stock` asks.
pub(crate) fn asking(job: JobId, store: Store) -> (Asking, Answering) {
    let (sender, events) = mpsc::channel();
    let answering = Answering {
        sender: sender.clone(),
    };
    let asking = Asking {
        job,
        store,
        sender,
        events,
    };
    (asking, answering)
}

impl Answering {
    /// Hand the job an answer the server has checked against its question
    /// and taken out of the state. A job that has ended no longer hears.
    pub(crate) fn send(&self, answer: Answer) {
        let _ = self.sender.send(Event::Answer(answer));
    }
}

impl Asking {
    fn ask(&self, pending: PendingPrompt) {
        self.store.update(|state| state.prompt = Some(pending));
    }

    /// Wait until the state shows the feeder session paused for this job,
    /// which `wellfed restore-stock` checks before it opens sessions of its own.
    fn until_paused(&self, cancel: &wellfed_rig::Cancel) -> Result<(), JobError> {
        let deadline = std::time::Instant::now() + PAUSE_WAIT;
        loop {
            if self.store.with(|state| state.own_sessions_job()) == Some(self.job) {
                return Ok(());
            }
            if cancel.is_cancelled() {
                return Err(wellfed_rig::Error::Interrupted.into());
            }
            if std::time::Instant::now() >= deadline {
                return Err(ToolJobError::NotPaused { job: self.job }.into());
            }
            std::thread::sleep(TICK);
        }
    }

    /// Take this job's question out of the state, if one is there.
    pub(crate) fn withdraw(&self) {
        let job = self.job;
        self.store.update(|state| {
            if state
                .prompt
                .as_ref()
                .is_some_and(|prompt| prompt.job == job)
            {
                state.prompt = None;
            }
        });
    }
}

/// Check `answer` against job `job`'s question `seq` in `prompt`, taking
/// the question out when it fits, so the same `seq` is never answered
/// twice.
pub(crate) fn take(
    prompt: &mut Option<PendingPrompt>,
    job: JobId,
    seq: u32,
    answer: Answer,
) -> Result<(), AnswerError> {
    let Some(pending) = prompt.as_ref().filter(|pending| pending.job == job) else {
        return Err(AnswerError::NoPrompt { job });
    };
    wellfed_formats::require(pending.seq == seq, || AnswerError::WrongSeq {
        job,
        seq,
        pending: pending.seq,
    })?;
    wellfed_formats::require(pending.prompt.accepts(answer), || AnswerError::Mismatched {
        name: pending.prompt.name().to_string(),
        expects: pending.expects,
        answer,
    })?;
    *prompt = None;
    Ok(())
}

/// Read `pipe` line by line into `events`, each line made an event by
/// `read`, and say when it ends. Lines are read as bytes and shown
/// lossily, so no output ends the reading early.
fn read_lines(
    pipe: impl Read + Send + 'static,
    events: mpsc::Sender<Event>,
    read: fn(String) -> Event,
) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut line = Vec::new();
        while matches!(reader.read_until(b'\n', &mut line), Ok(n) if n > 0) {
            let text = String::from_utf8_lossy(&line).trim_end().to_string();
            if events.send(read(text)).is_err() {
                return;
            }
            line.clear();
        }
        let _ = events.send(Event::Closed);
    });
}

fn prompt_line(line: String) -> Event {
    match serde_json::from_str::<Prompt>(&line) {
        Ok(prompt) => Event::Prompt(prompt),
        Err(_) => Event::Garbled(line),
    }
}

/// The tool's own error once it stops. Its `main` returns every failure, so the
/// error is the last block on its standard error: an `Error: ` line and anyhow's
/// `Caused by:` list. Any line after a block that is not part of anyhow's shape
/// means it was not the last word.
#[derive(Default)]
struct LastWords {
    lines: Vec<String>,
}

impl LastWords {
    fn hear(&mut self, line: &str) {
        if let Some(message) = line.strip_prefix("Error: ") {
            self.lines = vec![message.to_string()];
        } else if line.is_empty() || line == "Caused by:" || line.starts_with(' ') {
            if !self.lines.is_empty() {
                self.lines.push(line.to_string());
            }
        } else {
            self.lines.clear();
        }
    }

    fn message(self) -> Option<String> {
        let text = self.lines.join("\n");
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }
}

/// Run `wellfed restore-stock` to its end. Only a whole restore says what
/// bank 1 holds: anything else may have stopped part-way through its write.
pub(crate) fn restore_stock(
    bench: &mut Bench,
    args: &RestoreStockArgs,
    tools: &ToolPaths,
    asking: &Asking,
) -> Result<u8, JobError> {
    let talked = converse(
        bench,
        tools,
        asking,
        args.arguments(),
        args.virtual_panel,
        "restore-stock",
    )?;
    let succeeded = talked.waited.as_ref().is_ok_and(|status| status.success());
    let content = if succeeded {
        Content::Stock
    } else {
        Content::Unknown
    };
    bench.wrote(Bank::One, content, Source::Install);
    talked.ended()
}

/// `wellfed` with `arguments`, spoken with until its pipes close.
fn converse(
    bench: &mut Bench,
    tools: &ToolPaths,
    asking: &Asking,
    arguments: Vec<OsString>,
    virtual_panel: bool,
    label: &str,
) -> Result<Talked, JobError> {
    let path = &tools.wellfed_bin;
    wellfed_formats::require(path.is_file(), || ToolJobError::WellfedMissing {
        path: path.clone(),
    })?;
    asking.until_paused(&bench.cancel)?;

    let mut command = Command::new(path);
    own_group_with_input(&mut command)
        .current_dir(&bench.root)
        // The checkout's own key and backups, whatever the host user's are.
        .env(
            "WELLFED_HOME",
            bench.root.join(wellfed_bench_formats::WELLFED_HOME),
        )
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(wellfed_rig::Error::from)?;
    let tracked = bench.children.track(&child, ActivityKind::Install, label);

    if let Some(stdout) = child.stdout.take() {
        read_lines(stdout, asking.sender.clone(), prompt_line);
    }
    if let Some(stderr) = child.stderr.take() {
        read_lines(stderr, asking.sender.clone(), Event::Output);
    }
    let mut talk = Conversation {
        virtual_panel,
        input: child.stdin.take(),
        seq: 0,
        stopping: None,
        cancelled: false,
        last_words: LastWords::default(),
        backlog: VecDeque::new(),
    };
    talk.run(bench, asking, &mut child);

    // Every pipe has closed, so the tool has exited or is about to.
    let waited = child.wait();
    drop(tracked);
    asking.withdraw();
    Ok(Talked { waited, talk })
}

/// The tool has exited, and the job's talk with it is over.
struct Talked {
    waited: std::io::Result<std::process::ExitStatus>,
    talk: Conversation,
}

impl Talked {
    /// How the job ends, from why it stopped the tool and how the tool exited.
    fn ended(self) -> Result<u8, JobError> {
        let Self { waited, talk } = self;
        let status = waited.map_err(wellfed_rig::Error::from)?;

        if let Some(error) = talk.stopping {
            return Err(error);
        }
        if talk.cancelled {
            return Err(wellfed_rig::Error::Interrupted.into());
        }
        match status.code() {
            Some(0) => Ok(0),
            Some(code) => Err(ToolJobError::ToolStopped {
                code,
                message: talk.last_words.message(),
            }
            .into()),
            // A signal: whatever it printed last was not its own error.
            None => Err(ToolJobError::ToolStopped {
                code: 1,
                message: None,
            }
            .into()),
        }
    }
}

/// One tool's questions and answers, until its pipes close.
struct Conversation {
    /// The job answers the panel's prompt with its press.
    virtual_panel: bool,
    input: Option<ChildStdin>,
    /// The number of the last question passed on.
    seq: u32,
    /// Why the job stopped the tool, once it has.
    stopping: Option<JobError>,
    cancelled: bool,
    last_words: LastWords,
    /// Events read while the job pressed, to handle before any new one.
    backlog: VecDeque<Event>,
}

/// How the job's press for a panel prompt ended, short of an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pressed {
    /// The press echoed and released: the prompt is answered done.
    Done,
    /// The job did not press, as the tool ended or the job is stopping: no
    /// answer, and nothing is pressed.
    Abandoned,
}

impl Conversation {
    fn run(&mut self, bench: &mut Bench, asking: &Asking, child: &mut Child) {
        let mut open = 2;
        while open > 0 {
            // A cancel signals the tool itself (`Children::interrupt`), which
            // exits through its own stop; its question goes.
            if !self.cancelled && bench.cancel.is_cancelled() {
                self.cancelled = true;
                asking.withdraw();
            }
            let received = match self.backlog.pop_front() {
                Some(event) => Ok(event),
                None => asking.events.recv_timeout(TICK),
            };
            let event = match received {
                Ok(event) => event,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                // The job holds a sender itself, so this never comes.
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };

            match event {
                Event::Output(line) => {
                    eprintln!("{line}");
                    bench.status.line(&line);
                    self.last_words.hear(&line);
                }
                Event::Closed => open -= 1,
                Event::Garbled(line) => {
                    eprintln!("{line}");
                    self.stop(bench, asking, ToolJobError::Garbled { line }.into());
                }
                // The tool is being stopped: nothing it asks now is put to
                // anyone.
                Event::Prompt(_) if self.cancelled || self.stopping.is_some() => {}
                Event::Prompt(prompt) if self.virtual_panel && panel_press(&prompt).is_some() => {
                    match self.press_for(bench, asking, child, &prompt) {
                        Ok(Pressed::Done) => self.reply(Answer::Done),
                        Ok(Pressed::Abandoned) => {}
                        // A cancel interrupted the press: the loop's own
                        // check stops the tool.
                        Err(_) if bench.cancel.is_cancelled() => {}
                        Err(error) => self.stop(bench, asking, error),
                    }
                }
                Event::Prompt(prompt) => {
                    self.seq += 1;
                    let pending = PendingPrompt::new(asking.job, self.seq, prompt);
                    wellfed_rig::say!("Job {} asks ({}): {}", asking.job, self.seq, pending.text);
                    wellfed_rig::say!(
                        "Answer with: wellfed-bench answer {} {} {}",
                        asking.job,
                        self.seq,
                        pending.answer_words()
                    );
                    asking.ask(pending);
                }
                // The server passes an answer only for the question in the
                // state, and this loop withdraws that before it stops the
                // tool, so every answer here was taken while the tool waited
                // for it: it goes to the tool, even one stopping by now.
                Event::Answer(answer) => {
                    wellfed_rig::say!("Answered {answer}");
                    self.reply(answer);
                }
            }
        }
    }

    /// Type `prompt`'s press to the virtual panel, recorded in the job's
    /// output. The job looks first: once it is stopping, or the tool has ended,
    /// nothing is pressed, since no feeder window waits for it any longer. A
    /// press that does not echo and release ends it, and is not typed again.
    fn press_for(
        &mut self,
        bench: &mut Bench,
        asking: &Asking,
        child: &mut Child,
        prompt: &Prompt,
    ) -> Result<Pressed, JobError> {
        let Some(chord) = panel_press(prompt) else {
            return Ok(Pressed::Abandoned);
        };
        if self.halted(bench, asking, child) {
            wellfed_rig::say!(
                "Virtual panel: no press for the {} prompt; the tool ended or the job is stopping",
                prompt.name()
            );
            return Ok(Pressed::Abandoned);
        }
        wellfed_rig::say!(
            "Virtual panel, for the {} prompt: press {chord}",
            prompt.name()
        );
        let pressed = bench.press_within(&press_name(), chord, DEFAULT_HOLD_MS)?;
        wellfed_formats::require(pressed, || ToolJobError::PressFailed {
            prompt: prompt.name(),
            chord,
        })?;
        Ok(Pressed::Done)
    }

    /// Whether the job must press nothing more: a cancel or a stop, or a tool
    /// whose pipes closed or that exited. Events read to know wait in the
    /// backlog, in order, for the loop.
    fn halted(&mut self, bench: &Bench, asking: &Asking, child: &mut Child) -> bool {
        if self.cancelled || self.stopping.is_some() || bench.cancel.is_cancelled() {
            return true;
        }
        while let Ok(event) = asking.events.try_recv() {
            self.backlog.push_back(event);
        }
        let closed = self
            .backlog
            .iter()
            .any(|event| matches!(event, Event::Closed));
        // `try_wait` keeps the status it reaps for the job's own `wait`.
        closed || !matches!(child.try_wait(), Ok(None))
    }

    /// Write `answer` as the tool's next input line. A closed pipe means it is
    /// exiting, which its pipes closing then shows.
    fn reply(&mut self, answer: Answer) {
        let Some(input) = &mut self.input else {
            return;
        };
        let written = serde_json::to_string(&answer)
            .map_err(std::io::Error::from)
            .and_then(|line| writeln!(input, "{line}"))
            .and_then(|()| input.flush());
        if let Err(error) = written {
            eprintln!("Could not answer wellfed restore-stock: {error}");
        }
    }

    /// Stop the tool as a cancel does, on its own SIGTERM handler, and keep
    /// reading until it has gone.
    fn stop(&mut self, bench: &Bench, asking: &Asking, error: JobError) {
        if self.stopping.is_none() {
            self.stopping = Some(error);
            asking.withdraw();
            bench.children.interrupt();
        }
    }
}

/// The chord that answers `prompt` on the panel, held for a press; `None` for a
/// prompt no press answers.
///
/// No Lock hold unlocks the panel first, though a person might: a waiting
/// enrollment takes the next Dispense on a locked panel too, and a locked panel
/// also means a stray Dispense outside the window feeds nothing.
fn panel_press(prompt: &Prompt) -> Option<Buttons> {
    match prompt {
        Prompt::PressDispense {
            purpose: DispensePurpose::EnrollUpdateKey,
        } => Some(Buttons::of(Button::Dispense)),
        Prompt::KeySetupCode { .. }
        | Prompt::RestartFeeder
        | Prompt::RecognizeFeeder { .. }
        | Prompt::SetupMode
        | Prompt::JoinBack
        | Prompt::OpenRecoveryWindow => None,
    }
}

#[cfg(test)]
mod tests {
    use wellfed_install::prompt::Expects;

    use super::*;

    #[test]
    fn an_answer_is_taken_once_for_its_own_question_and_kind() {
        let asked = PendingPrompt::new(4, 2, Prompt::OpenRecoveryWindow);
        assert_eq!(asked.expects, Expects::Done);
        let mut prompt = Some(asked);

        assert_eq!(
            take(&mut prompt, 3, 2, Answer::Done),
            Err(AnswerError::NoPrompt { job: 3 })
        );
        assert_eq!(
            take(&mut prompt, 4, 1, Answer::Done),
            Err(AnswerError::WrongSeq {
                job: 4,
                seq: 1,
                pending: 2
            })
        );
        assert!(matches!(
            take(&mut prompt, 4, 2, Answer::Yes),
            Err(AnswerError::Mismatched {
                answer: Answer::Yes,
                ..
            })
        ));
        assert!(prompt.is_some(), "a refused answer leaves the question");

        take(&mut prompt, 4, 2, Answer::Done).unwrap();
        assert_eq!(prompt, None);
        assert_eq!(
            take(&mut prompt, 4, 2, Answer::Done),
            Err(AnswerError::NoPrompt { job: 4 })
        );
    }

    #[test]
    fn the_tool_s_last_words_start_at_its_last_error() {
        let mut words = LastWords::default();
        for line in [
            "Starting the stock server",
            "Error: first",
            "Error: Stopped at pairing",
            "",
            "Caused by:",
            "    no answer",
        ] {
            words.hear(line);
        }

        assert_eq!(
            words.message().as_deref(),
            Some("Stopped at pairing\n\nCaused by:\n    no answer")
        );
        assert_eq!(LastWords::default().message(), None);

        // An error followed by more of the run is not it.
        let mut words = LastWords::default();
        for line in ["Error: first try failed", "Offering stock."] {
            words.hear(line);
        }
        assert_eq!(words.message(), None);
    }
}

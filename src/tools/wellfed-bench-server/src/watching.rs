//! A view of a running server for a client that only watches: its live
//! state, and the output of its job as it comes.
//!
//! [`Watched`] holds no [`Client`] a caller could reach, and what it does
//! with its own is the state, the events and the output, all GETs, so a
//! watcher cannot post a job, cancel one or answer a question. Dropping it
//! only stops reading.
//!
//! A server that goes away leaves the last state in place, marked as not
//! answering, and is tried again every 2 s until one answers. While the
//! flash client writes, the output is polled no faster than twice a second,
//! so a watcher keeps the bench host as quiet as the server's own console
//! does. A watcher may stay up for days, so a reader that stops taking lines
//! (a stalled terminal) pauses the fetching rather than growing memory; the
//! server keeps the job's output meanwhile.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use wellfed_formats::bench::live::{BenchState, JobEnd, JobId, Phase, Uart};
use wellfed_formats::bench::now;

use crate::api::{OUTPUT_PAGE, OutputPage, Refusal};
use crate::client::{Client, ClientError};

/// Between tries of a server that does not answer.
const RETRY: Duration = Duration::from_secs(2);
/// How long a connecting request waits for an answer. `status` gives a
/// stuck server as long.
const ANSWER: Duration = Duration::from_secs(5);
/// Between polls of the job's output while nothing writes flash.
const POLL: Duration = Duration::from_millis(250);
/// Between polls during a write: about 2 Hz, the server's console's rate.
const WRITING_POLL: Duration = Duration::from_millis(500);
/// The lines waiting for a reader: two pages, so a screen that drains them
/// a few times a second still catches up at full speed.
const WAITING_LINES: usize = 2 * OUTPUT_PAGE;

/// What the watcher knows of the server.
struct View {
    state: BenchState,
    answering: bool,
}

/// A running server, watched by two tasks until this is dropped.
pub struct Watched {
    view: watch::Receiver<View>,
    lines: mpsc::Receiver<String>,
    tasks: [JoinHandle<()>; 2],
}

impl Watched {
    /// Start watching the server `client` is for, on the current runtime.
    /// A server that does not answer now is an error, not a retry: a watcher
    /// is started for a server that runs.
    pub async fn start(client: Client) -> Result<Self, ClientError> {
        let state = tokio::time::timeout(ANSWER, client.state())
            .await
            .map_err(|_| ClientError::Silent { after: ANSWER })??;

        let client = Arc::new(client);
        let (send_view, view) = watch::channel(View {
            state,
            answering: true,
        });
        let (send_lines, lines) = mpsc::channel(WAITING_LINES);
        let tasks = [
            tokio::spawn(follow_state(Arc::clone(&client), send_view)),
            tokio::spawn(follow_output(client, view.clone(), send_lines)),
        ];
        Ok(Self { view, lines, tasks })
    }

    /// The last state the server sent, with this host's clock, so timers
    /// and the header keep moving between changes, as on its own console.
    pub fn state(&self) -> BenchState {
        let mut state = self.view.borrow().state.clone();
        state.now = now();
        state
    }

    /// Whether the server answered when last tried.
    pub fn answering(&self) -> bool {
        self.view.borrow().answering
    }

    /// The next line for the log, if one is waiting.
    pub fn line(&mut self) -> Option<String> {
        self.lines.try_recv().ok()
    }
}

impl Drop for Watched {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Keep `view` on the server's state, across its going and coming back.
async fn follow_state(client: Arc<Client>, view: watch::Sender<View>) {
    loop {
        let read = read_states(&client, &view).await;

        // A stopping server ends its stream having said so and may answer
        // for a while yet; that is shown as stopping. Only when it no longer
        // connects has it gone, or a stream cut with no such word.
        let stopping = view.borrow().state.server.phase == Phase::Stopping;
        if read.is_err() || !stopping {
            view.send_if_modified(|view| std::mem::replace(&mut view.answering, false));
        }

        tokio::time::sleep(RETRY).await;
    }
}

async fn read_states(client: &Client, view: &watch::Sender<View>) -> Result<(), ClientError> {
    let mut events = tokio::time::timeout(ANSWER, client.events())
        .await
        .map_err(|_| ClientError::Silent { after: ANSWER })??;
    while let Some(state) = events.next().await? {
        view.send_modify(|view| {
            view.state = state;
            view.answering = true;
        });
    }
    Ok(())
}

/// Hand each line of the job `view` names to `lines`, a poll at a time.
async fn follow_output(
    client: Arc<Client>,
    mut view: watch::Receiver<View>,
    lines: mpsc::Sender<String>,
) {
    let mut follower = Follower::default();
    loop {
        let mut out = Vec::new();
        let (read, writing) = {
            let view = view.borrow_and_update();
            let writing = matches!(view.state.uart, Uart::Writing { .. });
            (follower.next(&Seen::of(&view.state), &mut out), writing)
        };
        let pace = if writing { WRITING_POLL } else { POLL };

        // Nothing to read until the state names another job.
        let Some(read) = read else {
            if !send(&lines, out).await || view.changed().await.is_err() {
                return;
            }
            continue;
        };

        let wait = match tokio::time::timeout(ANSWER, client.output(read.job, read.after)).await {
            Ok(Ok(page)) => {
                // Catching up, the next page follows at once, but for a write.
                let full = page.lines.len() >= OUTPUT_PAGE;
                follower.page(read.job, page, &mut out);
                if full && !writing {
                    Duration::ZERO
                } else {
                    pace
                }
            }
            Ok(Err(ClientError::Refused(Refusal::NoSuchJob { .. }))) => {
                follower.lost(read.job, &mut out);
                pace
            }
            Ok(Err(_)) | Err(_) => RETRY,
        };
        if !send(&lines, out).await {
            return;
        }
        tokio::time::sleep(wait).await;
    }
}

/// Hand over `out`, waiting while the reader's queue is full, which holds
/// the next fetch back too. False once the watcher is gone.
async fn send(lines: &mpsc::Sender<String>, out: Vec<String>) -> bool {
    for line in out {
        if lines.send(line).await.is_err() {
            return false;
        }
    }
    true
}

/// What the follower needs of the state.
struct Seen<'a> {
    server: u32,
    /// The running job, or else the last to finish, with its label.
    job: Option<(JobId, &'a str)>,
}

impl<'a> Seen<'a> {
    fn of(state: &'a BenchState) -> Self {
        Self {
            server: state.server.pid,
            job: state.job.as_ref().map(|job| (job.id, job.label.as_str())),
        }
    }
}

/// A read to make of a job's output.
#[derive(Debug, PartialEq, Eq)]
struct Read {
    job: JobId,
    after: u64,
}

/// Where the log has got to, with no IO of its own: told what the state
/// says and what each page held, it says what to read next and which lines
/// the log gets.
#[derive(Default)]
struct Follower {
    server: Option<u32>,
    at: Option<Cursor>,
}

struct Cursor {
    job: JobId,
    after: u64,
    /// Its end is read, so there is nothing more to ask for.
    finished: bool,
}

impl Follower {
    /// What to read now that the state says `seen`, if anything, adding any
    /// separator to `out`.
    fn next(&mut self, seen: &Seen<'_>, out: &mut Vec<String>) -> Option<Read> {
        // Another server, or job numbers going back, make every cursor here
        // meaningless, even one with the same number.
        let restarted = self.server.is_some_and(|server| server != seen.server);
        let backward = match (&self.at, seen.job) {
            (Some(at), Some((job, _))) => job < at.job,
            _ => false,
        };
        if restarted || backward {
            self.at = None;
            out.push(format!("── server restarted: pid {} ──", seen.server));
        }
        self.server = Some(seen.server);

        // A newer job waits until the last has given up its lines and end,
        // so one that ends just as the next starts is not cut short.
        let free = self.at.as_ref().is_none_or(|at| at.finished);
        if let Some((job, label)) = seen.job.filter(|_| free)
            && self.at.as_ref().is_none_or(|at| at.job != job)
        {
            out.push(format!("── job {job}: {label} ──"));
            self.at = Some(Cursor {
                job,
                after: 0,
                finished: false,
            });
        }

        self.at.as_ref().filter(|at| !at.finished).map(|at| Read {
            job: at.job,
            after: at.after,
        })
    }

    /// Take the page read for `job`. One for a job a restart has left
    /// behind is dropped.
    fn page(&mut self, job: JobId, page: OutputPage, out: &mut Vec<String>) {
        let Some(at) = self.at.as_mut().filter(|at| at.job == job) else {
            return;
        };

        // A long job's oldest lines go first, so the log says what it lost.
        if page.from > at.after {
            let lost = page.from - at.after;
            out.push(format!(
                "── job {job}: {lost} earlier lines no longer kept ──"
            ));
        }
        out.extend(page.lines);
        at.after = page.next;

        if let Some(end) = page.end {
            at.finished = true;
            out.push(end_line(job, &end));
        }
    }

    /// The server no longer keeps `job`'s output at all.
    fn lost(&mut self, job: JobId, out: &mut Vec<String>) {
        if let Some(at) = self.at.as_mut().filter(|at| at.job == job) {
            at.finished = true;
            out.push(format!(
                "── job {job}: the rest of its output is no longer kept ──"
            ));
        }
    }
}

fn end_line(job: JobId, end: &JobEnd) -> String {
    match (end.code, &end.error) {
        (0, _) => format!("── job {job} complete ──"),
        (code, Some(error)) => format!("── job {job} stopped ({code}): {error} ──"),
        (code, None) => format!("── job {job} stopped ({code}) ──"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SERVER: u32 = 100;

    fn seen(job: Option<JobId>) -> Seen<'static> {
        Seen {
            server: SERVER,
            job: job.map(|job| (job, "capture c1")),
        }
    }

    fn page(from: u64, lines: &[&str], end: Option<u8>) -> OutputPage {
        OutputPage {
            from,
            lines: lines.iter().map(ToString::to_string).collect(),
            next: from + u64::try_from(lines.len()).unwrap(),
            end: end.map(|code| JobEnd {
                code,
                error: None,
                finished: 1.0,
            }),
        }
    }

    #[test]
    fn with_no_job_it_reads_nothing_and_a_new_one_gets_a_separator() {
        let mut follower = Follower::default();
        let mut out = Vec::new();

        assert_eq!(follower.next(&seen(None), &mut out), None);
        assert!(out.is_empty());

        let read = follower.next(&seen(Some(7)), &mut out);

        assert_eq!(read, Some(Read { job: 7, after: 0 }));
        assert_eq!(out, ["── job 7: capture c1 ──"]);
    }

    #[test]
    fn it_carries_on_from_each_page_and_stops_asking_at_the_jobs_end() {
        let mut follower = Follower::default();
        let mut out = Vec::new();
        follower.next(&seen(Some(7)), &mut out);
        out.clear();

        follower.page(7, page(0, &["one", "two"], None), &mut out);
        let read = follower.next(&seen(Some(7)), &mut out);
        assert_eq!(read, Some(Read { job: 7, after: 2 }));

        follower.page(7, page(2, &["three"], Some(0)), &mut out);
        assert_eq!(follower.next(&seen(Some(7)), &mut out), None);
        assert_eq!(out, ["one", "two", "three", "── job 7 complete ──"]);
    }

    #[test]
    fn a_job_that_failed_says_how_in_the_log() {
        let end = JobEnd {
            code: 3,
            error: Some("no banner".into()),
            finished: 1.0,
        };

        assert_eq!(end_line(7, &end), "── job 7 stopped (3): no banner ──");
    }

    #[test]
    fn a_newer_job_waits_for_the_end_of_the_one_being_read() {
        let mut follower = Follower::default();
        let mut out = Vec::new();
        follower.next(&seen(Some(7)), &mut out);
        follower.page(7, page(0, &["one"], None), &mut out);
        out.clear();

        // The state moved on, with job 7's end still unread.
        let read = follower.next(&seen(Some(8)), &mut out);
        assert_eq!(read, Some(Read { job: 7, after: 1 }));
        assert!(out.is_empty());

        follower.page(7, page(1, &["two"], Some(0)), &mut out);
        let read = follower.next(&seen(Some(8)), &mut out);

        assert_eq!(read, Some(Read { job: 8, after: 0 }));
        assert_eq!(
            out,
            ["two", "── job 7 complete ──", "── job 8: capture c1 ──"]
        );
    }

    #[test]
    fn output_trimmed_from_the_front_is_said_and_skipped() {
        let mut follower = Follower::default();
        let mut out = Vec::new();
        follower.next(&seen(Some(7)), &mut out);
        follower.page(7, page(0, &["one"], None), &mut out);
        out.clear();

        follower.page(7, page(50, &["fifty"], None), &mut out);

        assert_eq!(
            out,
            ["── job 7: 49 earlier lines no longer kept ──", "fifty"]
        );
        let read = follower.next(&seen(Some(7)), &mut out);
        assert_eq!(read, Some(Read { job: 7, after: 51 }));
    }

    #[test]
    fn attaching_to_a_long_job_begins_at_its_first_kept_line() {
        let mut follower = Follower::default();
        let mut out = Vec::new();
        follower.next(&seen(Some(7)), &mut out);
        out.clear();

        follower.page(7, page(1000, &["kept"], None), &mut out);

        assert_eq!(
            out,
            ["── job 7: 1000 earlier lines no longer kept ──", "kept"]
        );
    }

    #[test]
    fn a_job_number_going_back_starts_over() {
        let mut follower = Follower::default();
        let mut out = Vec::new();
        follower.next(&seen(Some(7)), &mut out);
        follower.page(7, page(0, &["old"], None), &mut out);
        out.clear();

        let read = follower.next(&seen(Some(2)), &mut out);

        assert_eq!(read, Some(Read { job: 2, after: 0 }));
        assert_eq!(
            out,
            ["── server restarted: pid 100 ──", "── job 2: capture c1 ──"]
        );
    }

    #[test]
    fn another_server_starts_over_even_with_the_same_job_number() {
        let mut follower = Follower::default();
        let mut out = Vec::new();
        follower.next(&seen(Some(7)), &mut out);
        follower.page(7, page(0, &["old"], Some(0)), &mut out);
        out.clear();

        let restarted = Seen {
            server: SERVER + 1,
            job: Some((7, "boot b1")),
        };
        let read = follower.next(&restarted, &mut out);

        assert_eq!(read, Some(Read { job: 7, after: 0 }));
        assert_eq!(
            out,
            ["── server restarted: pid 101 ──", "── job 7: boot b1 ──"]
        );

        // Told the same again, it does not start over again.
        out.clear();
        follower.next(&restarted, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_new_server_with_no_job_leaves_nothing_to_read() {
        let mut follower = Follower::default();
        let mut out = Vec::new();
        follower.next(&seen(Some(7)), &mut out);
        out.clear();

        let read = follower.next(
            &Seen {
                server: SERVER + 1,
                job: None,
            },
            &mut out,
        );

        assert_eq!(read, None);
        assert_eq!(out, ["── server restarted: pid 101 ──"]);
    }

    #[test]
    fn a_page_from_before_a_restart_is_dropped() {
        let mut follower = Follower::default();
        let mut out = Vec::new();
        follower.next(&seen(Some(7)), &mut out);
        follower.next(&seen(Some(2)), &mut out);
        out.clear();

        follower.page(7, page(0, &["old"], Some(0)), &mut out);

        assert!(out.is_empty());
        assert_eq!(
            follower.next(&seen(Some(2)), &mut out),
            Some(Read { job: 2, after: 0 })
        );
    }

    #[test]
    fn a_job_the_server_no_longer_keeps_gives_way_to_the_next() {
        let mut follower = Follower::default();
        let mut out = Vec::new();
        follower.next(&seen(Some(7)), &mut out);
        out.clear();

        follower.lost(7, &mut out);
        assert_eq!(
            out,
            ["── job 7: the rest of its output is no longer kept ──"]
        );
        assert_eq!(follower.next(&seen(Some(7)), &mut out), None);

        let read = follower.next(&seen(Some(8)), &mut out);
        assert_eq!(read, Some(Read { job: 8, after: 0 }));
    }
}

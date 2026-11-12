//! The `restore-stock` job against a stand-in for `wellfed`: shell scripts
//! that write prompt lines, record the answers they read and the signal
//! that stops them.

use wellfed_bench_formats::banks::{Bank, Content};
use wellfed_bench_formats::live::{AnswerError, PendingPrompt};
use wellfed_install::prompt::{Answer, Prompt};

use super::*;

/// A restore that answers its panel prompt itself.
const RESTORE: &[&str] = &["restore-stock", "--virtual-panel"];

/// Write `script` as the checkout's `name`, executable.
fn stand_in(checkout: &Checkout, name: &str, script: &str) {
    let path = checkout.root().join(name);
    fs::write(&path, script).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A tool that asks for a yes or no and then for something done, recording
/// each answer it reads in `answers`. The real one asks only for the Dispense
/// press; these two show each kind of answer on the way through.
const ASKS: &str = r#"#!/bin/sh
echo "$@" > args
echo "$WELLFED_HOME" > home
echo "Starting" >&2
echo '{"kind":"recognize-feeder","device":"AF_TEST","address":"192.168.1.31"}'
read a; echo "recognize $a" >> answers
echo '{"kind":"press-dispense","purpose":"enroll-update-key"}'
read a; echo "dispense $a" >> answers
echo "Restored" >&2
"#;

/// A tool that asks one question and waits, never reading the answer, until
/// a signal stops it.
///
/// It waits in short sleeps, never one long one: a TERM that reaches a
/// forked `sleep` before it execs is caught by the shell's handler it still
/// has and lost at the exec, and a 30 s sleep would then hold the job's
/// output open, and the test, for its whole length.
const WAITS: &str = r#"#!/bin/sh
trap 'echo TERM > signal; exit 1' TERM
echo '{"kind":"press-dispense","purpose":"enroll-update-key"}'
i=0; while [ $i -lt 300 ]; do sleep 0.1; i=$((i+1)); done
"#;

fn read(checkout: &Checkout, name: &str) -> String {
    fs::read_to_string(checkout.root().join(name)).unwrap_or_default()
}

async fn prompt_until(server: &Running, seq: u32) -> PendingPrompt {
    let state = state_until(server, "the question", |state| {
        state
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.seq == seq)
    })
    .await;
    state.prompt.unwrap()
}

fn refused(result: Result<(), ClientError>) -> AnswerError {
    match result {
        Err(ClientError::Refused(api::Refusal::Answer { error })) => error,
        other => panic!("not refused: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restore_job_asks_through_the_state() {
    if crate::server::tests::run_alone() {
        return;
    }
    let _one = ONE_SERVER.lock().await;
    let checkout = Checkout::new(0.0);
    stand_in(&checkout, "wellfed", ASKS);
    checkout.board.lock().on[1] = true;
    let server = Running::start(checkout.open());

    let id = server.submit(&["restore-stock"]).await.unwrap();

    let asked = prompt_until(&server, 1).await;
    assert_eq!(
        asked.prompt,
        Prompt::RecognizeFeeder {
            device: "AF_TEST".into(),
            address: "192.168.1.31".into()
        }
    );
    assert_eq!(
        (asked.job, asked.text.as_str()),
        (id, &*asked.prompt.text())
    );
    let state = server.state().await;
    assert_eq!(state.activity[0].kind, ActivityKind::Install);
    assert_eq!(state.job.unwrap().label, "restore-stock");

    // Another job, another number, another kind: each refused, and the
    // question stays.
    let client = &server.client;
    assert_eq!(
        refused(client.answer(id + 1, 1, Answer::Yes).await),
        AnswerError::NotRunning { job: id + 1 }
    );
    assert_eq!(
        refused(client.answer(id, 2, Answer::Yes).await),
        AnswerError::WrongSeq {
            job: id,
            seq: 2,
            pending: 1
        }
    );
    assert!(matches!(
        refused(client.answer(id, 1, Answer::Done).await),
        AnswerError::Mismatched {
            answer: Answer::Done,
            ..
        }
    ));
    assert!(server.state().await.prompt.is_some());
    assert_eq!(read(&checkout, "answers"), "");

    // The right answer reaches the tool, once.
    client.answer(id, 1, Answer::Yes).await.unwrap();
    let press = prompt_until(&server, 2).await;
    assert_eq!(press.prompt.name(), "press-dispense");
    assert!(matches!(
        refused(client.answer(id, 1, Answer::Yes).await),
        AnswerError::WrongSeq { pending: 2, .. }
    ));
    let waited = client
        .wait(&Wait {
            what: WaitFor::Prompt,
            id: None,
            text: None,
            kind: None,
            build: None,
            bank: None,
            label: None,
            timeout_ms: 1_000,
        })
        .await
        .unwrap();
    assert!(
        matches!(&waited, api::Waited::Met { detail } if detail.contains("press-dispense")),
        "{waited:?}"
    );
    client.answer(id, 2, Answer::Done).await.unwrap();

    let (end, lines) = follow(client, id).await;
    assert_eq!((end.code, end.error), (0, None));
    assert_eq!(
        read(&checkout, "answers"),
        "recognize \"yes\"\ndispense \"done\"\n"
    );
    assert!(lines.iter().any(|line| line == "Restored"), "{lines:?}");
    assert_eq!(
        read(&checkout, "home").trim(),
        checkout.root().join("private").display().to_string(),
        "the checkout's key and backups"
    );
    // Without the flag the press was a person's: the job typed nothing.
    assert_eq!(read(&checkout, "presses"), "");
    let state = server.state().await;
    assert!(state.prompt.is_none());
    assert_eq!(state.banks.banks[&Bank::One].content, Content::Stock);
    assert_eq!(
        refused(client.answer(id, 3, Answer::Done).await),
        AnswerError::NotRunning { job: id }
    );

    server.quit().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancel_stops_the_tool_and_takes_its_question_away() {
    if crate::server::tests::run_alone() {
        return;
    }
    let _one = ONE_SERVER.lock().await;
    let checkout = Checkout::new(0.0);
    stand_in(&checkout, "wellfed", WAITS);
    let server = Running::start(checkout.open());

    let id = server.submit(&["restore-stock"]).await.unwrap();
    prompt_until(&server, 1).await;
    server.client.cancel(id, DuringWrite::Refuse).await.unwrap();
    let (end, _) = follow(&server.client, id).await;

    assert_eq!(end.code, 1);
    assert_eq!(end.error.as_deref(), Some("Interrupted by the operator"));
    assert_eq!(read(&checkout, "signal").trim(), "TERM");
    let state = server.state().await;
    assert!(state.prompt.is_none() && state.activity.is_empty());
    // An answer that comes late reaches nothing.
    assert_eq!(
        refused(server.client.answer(id, 1, Answer::Done).await),
        AnswerError::NotRunning { job: id }
    );
    // It never needed relays to stop.
    assert!(checkout.board.log().is_empty());
    // The tool had started, so its write may have begun.
    assert_eq!(state.banks.banks[&Bank::One].content, Content::Unknown);

    server.quit().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_line_that_is_no_prompt_stops_the_tool_and_its_own_error_ends_the_job() {
    if crate::server::tests::run_alone() {
        return;
    }
    let _one = ONE_SERVER.lock().await;
    let checkout = Checkout::new(0.0);
    // It waits in short sleeps, as [`WAITS`] does and for the same reason.
    stand_in(
        &checkout,
        "wellfed",
        r#"#!/bin/sh
trap 'echo TERM > signal; exit 1' TERM
echo 'Press Dispense now'
i=0; while [ $i -lt 300 ]; do sleep 0.1; i=$((i+1)); done
"#,
    );
    let server = Running::start(checkout.open());

    let id = server.submit(&["restore-stock"]).await.unwrap();
    let (end, lines) = follow(&server.client, id).await;

    assert!(
        end.error
            .as_deref()
            .is_some_and(|error| error.contains("where a prompt goes")),
        "{end:?}"
    );
    assert!(lines.iter().any(|line| line == "Press Dispense now"));
    assert_eq!(read(&checkout, "signal").trim(), "TERM");
    assert!(server.state().await.prompt.is_none());

    server.quit().await;
}

/// `restore-stock` presses Dispense once for the tool's announcement, answers
/// it done, and records stock in bank 1 when the tool ends well. The flag stays
/// the server's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restore_job_presses_dispense_once_and_records_stock() {
    if crate::server::tests::run_alone() {
        return;
    }
    let _one = ONE_SERVER.lock().await;
    let checkout = Checkout::new(0.0);
    stand_in(
        &checkout,
        "wellfed",
        r#"#!/bin/sh
echo "$@" > args
echo '{"kind":"press-dispense","purpose":"enroll-update-key"}'
read a; echo "dispense $a" >> answers
"#,
    );
    checkout.board.lock().on[1] = true;
    let server = Running::start(checkout.open());

    let id = server.submit(RESTORE).await.unwrap();
    let (end, lines) = follow(&server.client, id).await;

    assert_eq!((end.code, end.error), (0, None), "{lines:?}");
    assert_eq!(read(&checkout, "presses"), "dispense 150\n");
    assert_eq!(read(&checkout, "answers"), "dispense \"done\"\n");
    assert_eq!(
        read(&checkout, "args").trim(),
        "restore-stock --prompts=lines"
    );
    let bank = server.state().await.banks.banks[&Bank::One].clone();
    assert_eq!(bank.content, Content::Stock);
    assert!(server.state().await.prompt.is_none());

    server.quit().await;
}

/// A press that does not echo stops the tool, nothing is pressed again, and
/// bank 1 is unknown; an uncertain restore's own words end the job as they were.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restore_job_s_failed_press_stops_it_and_its_uncertain_words_end_it() {
    if crate::server::tests::run_alone() {
        return;
    }
    let _one = ONE_SERVER.lock().await;
    let checkout = Checkout::new(0.0);
    stand_in(&checkout, "wellfed", WAITS);
    fs::write(checkout.root().join("press-fails"), "dispense").unwrap();
    checkout.board.lock().on[1] = true;
    let server = Running::start(checkout.open());

    let id = server.submit(RESTORE).await.unwrap();
    let (end, lines) = follow(&server.client, id).await;

    assert_eq!(end.code, 1);
    assert!(
        end.error
            .as_deref()
            .is_some_and(|error| error.contains("press-dispense") && error.contains("(dispense)")),
        "{end:?}"
    );
    assert_eq!(read(&checkout, "presses"), "dispense 150\n");
    assert_eq!(read(&checkout, "signal").trim(), "TERM");
    // The job's press adds no step row of its own, and the job's error, not a
    // press job's "stopped" line, says what the failure means.
    let record = server.state().await.job.unwrap().record.unwrap();
    assert!(
        record
            .steps
            .iter()
            .all(|step| !step.label.contains("read press")),
        "{:?}",
        record.steps
    );
    assert!(
        !lines
            .iter()
            .any(|line| line.contains("Stopped without retry")),
        "{lines:?}"
    );
    assert_eq!(read(&checkout, "answers"), "");
    assert!(server.state().await.prompt.is_none());
    assert_eq!(
        server.state().await.banks.banks[&Bank::One].content,
        Content::Unknown
    );

    stand_in(
        &checkout,
        "wellfed",
        "#!/bin/sh\necho \"Error: uncertain: the feeder stopped answering\" >&2\nexit 1\n",
    );
    let id = server.submit(&["restore-stock"]).await.unwrap();
    let (end, _) = follow(&server.client, id).await;
    assert_eq!(end.code, 1);
    assert_eq!(
        end.error.as_deref(),
        Some("uncertain: the feeder stopped answering")
    );
    assert_eq!(
        server.state().await.banks.banks[&Bank::One].content,
        Content::Unknown
    );

    server.quit().await;
}

/// A tool that ends as it announces the press gets no answer: the job ends in
/// the tool's own words, with at most the one press that was already under way.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tool_that_has_ended_gets_no_more_than_the_press_under_way() {
    if crate::server::tests::run_alone() {
        return;
    }
    let _one = ONE_SERVER.lock().await;
    let checkout = Checkout::new(0.0);
    stand_in(
        &checkout,
        "wellfed",
        r#"#!/bin/sh
echo '{"kind":"press-dispense","purpose":"enroll-update-key"}'
echo "Error: the feeder refused the enrollment" >&2
exit 1
"#,
    );
    checkout.board.lock().on[1] = true;
    let server = Running::start(checkout.open());

    let id = server.submit(RESTORE).await.unwrap();
    let (end, _) = follow(&server.client, id).await;

    assert_eq!(
        (end.code, end.error.as_deref()),
        (1, Some("the feeder refused the enrollment"))
    );
    let presses = read(&checkout, "presses");
    assert!(presses.lines().count() <= 1, "{presses}");

    server.quit().await;
}

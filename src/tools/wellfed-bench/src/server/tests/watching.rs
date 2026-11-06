//! The event stream and a [`Watched`] server over a real socket.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use axum::extract::Request;
use axum::http::Method;
use axum::middleware::{Next, from_fn};
use tokio::net::UnixListener;

use super::*;
use crate::server::Watched;

type Seen = Arc<Mutex<Vec<(Method, String)>>>;

/// A socket that serves `server`'s own routes and records what each request
/// asked, which is what a client's read-only claim comes down to.
fn recorded(server: &Running, dir: &Path) -> (PathBuf, Seen) {
    let seen = Seen::default();
    let recording = Arc::clone(&seen);
    let router = crate::server::routes::router(Arc::clone(&server.shared)).layer(from_fn(
        move |request: Request, next: Next| {
            let recording = Arc::clone(&recording);
            async move {
                recording
                    .lock()
                    .unwrap()
                    .push((request.method().clone(), request.uri().path().to_string()));
                next.run(request).await
            }
        },
    ));
    let socket = dir.join("recorded.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    (socket, seen)
}

/// Take the lines `watched` has until one is `line`.
async fn read_until(watched: &mut Watched, kept: &mut Vec<String>, line: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !kept.iter().any(|kept| kept == line) {
        match watched.line() {
            Some(next) => kept.push(next),
            None => {
                assert!(Instant::now() < deadline, "no {line:?} in {kept:?}");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_event_stream_sends_the_state_each_change_and_ends_when_the_server_stops() {
    let Some(_turn) = turn().await else { return };
    let checkout = Checkout::new(0.1);
    let server = Running::start(checkout.open());
    let client = Client::new(&server.root);

    let mut events = client.events().await.unwrap();
    let first = events.next().await.unwrap().unwrap();
    assert_eq!(first.server.phase, Phase::Serving);
    assert_eq!(first.server.pid, std::process::id());

    let id = server.submit(&["capture", "c1"]).await.unwrap();
    loop {
        let state = events.next().await.unwrap().unwrap();
        if state
            .job
            .is_some_and(|job| job.id == id && job.end.is_some())
        {
            break;
        }
    }

    server.quit().await;
    let mut last = None;
    while let Some(state) = events.next().await.unwrap() {
        last = Some(state);
    }
    assert_eq!(last.unwrap().server.phase, Phase::Stopping);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watcher_follows_a_job_with_gets_alone_and_leaving_lets_it_run_on() {
    let Some(_turn) = turn().await else { return };
    let checkout = Checkout::new(4.0);
    let server = Running::start(checkout.open());
    let (socket, seen) = recorded(&server, &checkout.root());
    let id = server.submit(&["capture", "c1"]).await.unwrap();
    let separator = format!("── job {id}: capture c1 ──");

    // Leave in the middle of the job.
    let mut watched = Watched::start(Client::at(socket.clone())).await.unwrap();
    let mut kept = Vec::new();
    read_until(&mut watched, &mut kept, &separator).await;
    assert!(watched.answering());
    assert_eq!(watched.state().job.map(|job| job.id), Some(id));
    drop(watched);

    // The job's thread sets its activity a moment after the job is accepted.
    let state = state_until(&server, "the capture", |state| !state.activity.is_empty()).await;
    assert_eq!(state.server.phase, Phase::Serving);
    let job = state.job.as_ref().unwrap();
    assert!(
        job.end.is_none() && job.cancel == CancelState::None,
        "{job:?}"
    );

    // Another attaches, and reads the job to its end.
    let mut watched = Watched::start(Client::at(socket)).await.unwrap();
    let mut kept = Vec::new();
    read_until(&mut watched, &mut kept, &format!("── job {id} complete ──")).await;
    assert_eq!(kept.iter().filter(|line| **line == separator).count(), 1);
    assert!(
        kept.contains(&"Captured 7 bytes in private/captures/c1".to_string()),
        "{kept:?}"
    );
    drop(watched);
    assert_eq!(follow(&server.client, id).await.0.code, 0);

    // Every request was a GET of the three routes a watcher reads.
    let seen = seen.lock().unwrap().clone();
    let output = format!("/jobs/{id}/output");
    assert!(seen.iter().any(|(_, path)| *path == output), "{seen:?}");
    assert!(seen.iter().any(|(_, path)| path == "/events"), "{seen:?}");
    for (method, path) in &seen {
        assert_eq!(method, Method::GET, "{path}");
        assert!(
            path == "/state" || path == "/events" || *path == output,
            "{path}"
        );
    }

    server.quit().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watcher_keeps_the_last_state_of_a_server_gone_and_resyncs_with_the_next() {
    let Some(_turn) = turn().await else { return };
    let checkout = Checkout::new(0.2);
    let server = Running::start(checkout.open());
    let mut watched = Watched::start(Client::new(&server.root)).await.unwrap();

    let first = server.submit(&["capture", "c1"]).await.unwrap();
    let mut kept = Vec::new();
    read_until(
        &mut watched,
        &mut kept,
        &format!("── job {first} complete ──"),
    )
    .await;

    server.quit().await;
    until("the server to stop answering", || !watched.answering()).await;
    let last = watched.state();
    assert_eq!(last.server.phase, Phase::Stopping);
    assert_eq!(last.job.map(|job| job.id), Some(first));

    let server = Running::start(checkout.open());
    until("the next server to answer", || watched.answering()).await;
    assert_eq!(watched.state().server.phase, Phase::Serving);

    let second = server.submit(&["capture", "c2"]).await.unwrap();
    read_until(
        &mut watched,
        &mut kept,
        &format!("── job {second} complete ──"),
    )
    .await;

    drop(watched);
    server.quit().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_watcher_nobody_reads_stops_fetching_and_loses_nothing_once_read_again() {
    let Some(_turn) = turn().await else { return };
    let checkout = Checkout::new(0.0);
    let server = Running::start(checkout.open());
    let (socket, seen) = recorded(&server, &checkout.root());
    let mut watched = Watched::start(Client::at(socket)).await.unwrap();

    // 3000 lines against a queue of 1000, with nothing taking them. The
    // capture runs until it is released, so it is still running however
    // long the watcher takes to stop.
    let id = server.submit(&["capture", "held-burst"]).await.unwrap();
    let output = format!("/jobs/{id}/output");
    let polls = || {
        let seen = seen.lock().unwrap();
        seen.iter().filter(|(_, path)| *path == output).count()
    };

    // The job runs on, so a follower that was not held back would keep
    // polling four times a second.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let before = polls();
        tokio::time::sleep(Duration::from_secs(1)).await;
        if polls() == before && before > 0 {
            break;
        }
        assert!(Instant::now() < deadline, "still polling: {before}");
    }
    let job = server.state().await.job.unwrap();
    assert!(job.id == id && job.end.is_none(), "{job:?}");
    checkout.release("held-burst");

    let mut kept = Vec::new();
    read_until(&mut watched, &mut kept, &format!("── job {id} complete ──")).await;
    let burst = kept.iter().filter(|line| line.starts_with("burst line"));
    assert_eq!(burst.count(), 3000);

    drop(watched);
    server.quit().await;
}

#[test]
fn a_keep_alive_or_another_event_carries_no_state() {
    use crate::server::client::state_event;

    assert!(state_event(b": keep-alive").unwrap().is_none());
    assert!(state_event(b"event: other\ndata: {}").unwrap().is_none());
    assert!(state_event(b"event: state").unwrap().is_none());
    assert!(state_event(b"event: state\ndata: {").is_err());

    let data = serde_json::to_string(&idle_state()).unwrap();
    let event = format!("event: state\ndata: {data}");
    let state = state_event(event.as_bytes()).unwrap().unwrap();
    assert_eq!(state.server.pid, 1);
}

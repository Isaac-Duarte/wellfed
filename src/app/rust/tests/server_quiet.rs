//! The bridge's server session against addresses that misbehave, each a
//! bare TCP listener in the test (simulation, not hardware):
//! - a stream whose server goes quiet after its headers ends as
//!   `Interrupted` once the read timeout passes, instead of never;
//! - a plain call to a server that never answers ends the same way;
//! - a redirect is never followed, so the password goes nowhere else, and
//!   comes back as `Redirected` naming where to log in instead.

use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::LocalSet;
use wellfed_app_bridge::api::server::{ServerSession, ServerUpdate, Timeouts};
use wellfed_app_bridge::api::server_error::ServerErrorKind;
use wellfed_app_bridge::api::server_views::ServerLink;

/// Short enough for a test, long enough that a loaded host still connects.
const QUIET: Duration = Duration::from_millis(500);
const WAIT: Duration = Duration::from_secs(20);

fn local(test: impl Future<Output = ()>) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(LocalSet::new().run_until(test));
}

/// A listener that reads each request's head, writes the answer `answer`
/// makes of its own URL, and then holds the connection open in silence. It
/// counts the requests.
async fn fake(answer: impl Fn(&str) -> String) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let answer = answer(&url);
    let requests = Arc::new(AtomicUsize::new(0));

    let counted = Arc::clone(&requests);
    tokio::task::spawn_local(async move {
        let mut held = Vec::new();
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_head(&mut socket).await;
            counted.fetch_add(1, Ordering::SeqCst);
            socket.write_all(answer.as_bytes()).await.unwrap();
            held.push(socket);
        }
    });
    (url, requests)
}

async fn read_head(socket: &mut TcpStream) {
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if socket.read(&mut byte).await.unwrap() == 0 {
            return;
        }
        head.push(byte[0]);
    }
}

fn session(url: String) -> ServerSession {
    let link = ServerLink {
        url,
        key: "00".repeat(32),
        user: "ada".into(),
        token_id: "0192c5d0-0000-7000-8000-000000000001".into(),
    };
    let timeouts = Timeouts {
        read: Some(QUIET),
        request: Some(QUIET),
    };
    ServerSession::restore_with(link, "secret".into(), timeouts).unwrap()
}

#[test]
fn a_stream_that_goes_quiet_after_its_headers_ends_interrupted() {
    local(async {
        let (url, _) = fake(|_| {
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n".into()
        })
        .await;
        let session = Rc::new(session(url));

        let (sender, mut updates) = mpsc::unbounded_channel();
        let following = Rc::clone(&session);
        tokio::task::spawn_local(async move {
            following
                .follow_with(None, move |update| sender.send(update).is_ok())
                .await;
        });

        let heard = tokio::time::timeout(WAIT, async {
            let mut opened = false;
            loop {
                match updates.recv().await.expect("a follow ends with Closed") {
                    ServerUpdate::Opened => opened = true,
                    ServerUpdate::Closed { error } => return (opened, error.map(|e| e.kind)),
                    ServerUpdate::Event { .. } => panic!("the fake sends no events"),
                }
            }
        })
        .await
        .expect("a quiet stream ends");
        assert_eq!(heard, (true, Some(ServerErrorKind::Interrupted)));
    });
}

#[test]
fn a_call_the_server_never_answers_ends_interrupted() {
    local(async {
        // Its head read, the request gets nothing back.
        let (url, requests) = fake(|_| String::new()).await;
        let session = session(url);

        let result = tokio::time::timeout(WAIT, session.feeders())
            .await
            .expect("a stalled call ends");

        assert_eq!(
            result.map_err(|error| error.kind).err(),
            Some(ServerErrorKind::Interrupted)
        );
        assert_eq!(requests.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn a_redirect_is_not_followed_and_names_the_address_to_use() {
    local(async {
        // Back to this same listener, so a followed redirect would count.
        let (url, requests) = fake(|url| {
            format!(
                "HTTP/1.1 308 Permanent Redirect\r\nlocation: {url}/elsewhere\r\ncontent-length: 0\r\n\r\n"
            )
        })
        .await;
        let elsewhere = format!("{url}/elsewhere");

        let result = ServerSession::login(
            url,
            "ada".into(),
            "correct horse".into(),
            "pixel".into(),
            None,
        )
        .await;

        let Err(error) = result else {
            panic!("a redirect logged in");
        };
        assert_eq!(error.kind, ServerErrorKind::Redirected);
        assert!(error.message.contains(&elsewhere), "{error:?}");
        assert_eq!(
            requests.load(Ordering::SeqCst),
            1,
            "only the first request, to the address given"
        );
    });
}

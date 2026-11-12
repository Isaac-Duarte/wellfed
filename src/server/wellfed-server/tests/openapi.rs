//! `openapi.json` is committed, so another client's generated types build
//! without Rust and a review sees every contract change. Every route lists
//! the answers the checks in front of it give. This test fails
//! when it is stale; `UPDATE_OPENAPI=1 cargo test -p wellfed-server
//! --test openapi` rewrites it.

use std::path::PathBuf;

use utoipa::openapi::RefOr;
use wellfed_server_client::{Method, StatusCode};
use wellfed_server_testkit::TestServer;

#[test]
fn openapi_json_is_current() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("openapi.json");
    let mut want = wellfed_server::openapi()
        .to_pretty_json()
        .expect("the document serializes");
    want.push('\n');

    if std::env::var_os("UPDATE_OPENAPI").is_some() {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        have == want,
        "{} is stale; run UPDATE_OPENAPI=1 cargo test -p wellfed-server --test openapi",
        path.display()
    );
}

/// Every route documents the answers the checks in front of it give: the
/// host check's everywhere, the origin check's on every write, and the
/// login check's wherever a request without one is refused. That last is
/// asked of the server itself: each operation is sent with no login, and
/// it answers `401` exactly where the document lists it.
/// The login check's refusal, as the document words it.
const UNAUTHENTICATED: &str = "No valid login or token: `unauthenticated`";

#[tokio::test]
async fn every_route_documents_the_answers_it_shares() {
    let server = TestServer::start().await;
    let client = server.client();
    let doc = wellfed_server::openapi();

    for (path, item) in &doc.paths.paths {
        let operations = [
            (Method::GET, &item.get),
            (Method::PUT, &item.put),
            (Method::POST, &item.post),
            (Method::DELETE, &item.delete),
            (Method::PATCH, &item.patch),
        ];
        for (method, operation) in operations {
            let Some(operation) = operation else {
                continue;
            };
            let has = |status: &str| operation.responses.responses.contains_key(status);
            assert!(has("421"), "{method} {path} lists unknown_host");
            if method != Method::GET && path.starts_with("/api/v1/") {
                assert!(has("403"), "{method} {path} lists cross_origin");
            }

            // Every path parameter is an id but a feed's number.
            let concrete = path
                .replace("{feed}", "1")
                .split('/')
                .map(|part| {
                    if part.starts_with('{') {
                        "00000000-0000-0000-0000-000000000000"
                    } else {
                        part
                    }
                })
                .collect::<Vec<_>>()
                .join("/");
            let answer = client
                .request(method.clone(), &concrete)
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap();
            // A route may list a 401 of its own (a login's wrong password);
            // the login check's is the one the document adds everywhere.
            let refused = answer.status() == StatusCode::UNAUTHORIZED;
            let shared = match operation.responses.responses.get("401") {
                Some(RefOr::T(response)) => response.description == UNAUTHENTICATED,
                _ => false,
            };
            assert!(
                !refused || has("401"),
                "{method} {path} lists unauthenticated"
            );
            assert!(
                !shared || refused,
                "{method} {path} lists unauthenticated, and answers without a login"
            );
        }
    }
    server.stop().await;
}

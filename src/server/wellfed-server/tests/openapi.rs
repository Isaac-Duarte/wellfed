//! `openapi.json` is committed, so another client's generated types build
//! without Rust and a review sees every contract change. Every route lists
//! the answers the checks in front of it give. This test fails
//! when it is stale; `UPDATE_OPENAPI=1 cargo test -p wellfed-server
//! --test openapi` rewrites it.

use std::path::PathBuf;

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

/// Every route behind a login documents its refusal of a request without
/// one, and every route the host check's: answers a client must expect
/// whichever route it calls.
#[test]
fn every_route_documents_the_answers_it_shares() {
    let doc = wellfed_server::openapi();
    let open = [
        ("/api/v1/setup", "get"),
        ("/api/v1/setup", "post"),
        ("/api/v1/import", "post"),
        ("/api/v1/session", "post"),
    ];
    for (path, item) in &doc.paths.paths {
        let operations = [
            ("get", &item.get),
            ("put", &item.put),
            ("post", &item.post),
            ("delete", &item.delete),
            ("patch", &item.patch),
        ];
        for (method, operation) in operations {
            let Some(operation) = operation else {
                continue;
            };
            let has = |status: &str| operation.responses.responses.contains_key(status);
            assert!(has("421"), "{method} {path} lists unknown_host");
            let behind_login =
                path.starts_with("/api/v1/") && !open.contains(&(path.as_str(), method));
            assert!(
                !behind_login || has("401"),
                "{method} {path} lists unauthenticated"
            );
            if method != "get" && path.starts_with("/api/v1/") {
                assert!(has("403"), "{method} {path} lists cross_origin");
            }
        }
    }
}

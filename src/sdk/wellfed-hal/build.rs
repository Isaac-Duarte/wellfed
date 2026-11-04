//! Exposes the reviewed build wrapper's source identity as `WELLFED_BUILD_ID`,
//! and forwards the board `wellfed-board` chose, which gates the drivers only
//! one board can run.

#[path = "../wellfed-board/build/forward.rs"]
mod board;

fn main() {
    board::forward();
    println!("cargo:rerun-if-env-changed=WELLFED_BUILD_ID");
    // The built-in update-signing key (`ota::update::public_key`).
    println!("cargo:rerun-if-env-changed=WELLFED_OTA_PUBLIC_KEY");

    let identity = std::env::var("WELLFED_BUILD_ID").unwrap_or_else(|_| "host".into());
    assert!(
        identity == "host"
            || (identity.len() == 64
                && identity
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
    );
    assert!(
        std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("none") || identity != "host",
        "target builds require the reviewed build wrapper's identity"
    );

    println!("cargo:rustc-env=WELLFED_BUILD_ID={identity}");
}

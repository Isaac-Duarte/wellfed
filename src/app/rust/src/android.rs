//! The JNI hooks Kotlin calls with no Dart running.
//!
//! `MainActivity` and `BackgroundEngine` call the first before their engine
//! runs Dart, so a process WorkManager or the boot receiver started, with
//! no activity, has it too; a second call changes nothing. It gives the TLS
//! certificate verifier the app's context, so an `https` server is checked
//! against Android's own trust store, a CA the owner installed on the phone
//! included. reqwest's rustls asks that verifier, which panics on the first
//! check if it was never given the context.
//!
//! The Kotlin half of the verifier ships in the APK from the
//! `org.rustls:rustls-platform-verifier` Maven package, at the version the
//! lockfile pins for `rustls-platform-verifier-android`.
//!
//! `KeepAliveService` calls the second when the owner swipes the app's task
//! away: the engine's holds on catch-ups go with the engine, since none of
//! its runs is left to let go. A run's thread that still lives (an update
//! pushing chunks or waiting out the restart, an install's listener) keeps
//! its own hold until it ends.
//!
//! `Uplink` calls the third whenever the network to reach the owner's server
//! by changes: the default network's handle while the Wi-Fi the process is
//! bound to has no validated internet, zero otherwise (`relay.rs`).

use jni::EnvUnowned;
use jni::errors::ThrowRuntimeExAndDefault;
use jni::objects::{JClass, JObject};

/// `TlsVerifier.start(context)`, a static native method.
// SAFETY: the JVM resolves `TlsVerifier.start` to this exact name, and the
// signature is a static method's (`JNIEnv*`, `jclass`, `jobject`): no other
// symbol in the process carries it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isaacduarte_wellfed_TlsVerifier_start<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    context: JObject<'caller>,
) {
    // A failure throws in Kotlin, at startup, rather than at the first login.
    env.with_env(|env| rustls_platform_verifier::android::init_with_env(env, context))
        .resolve::<ThrowRuntimeExAndDefault>();
}

/// `RustHolds.clear()`, a static native method.
// SAFETY: the JVM resolves `RustHolds.clear` to this exact name, and the
// signature is a static method's (`JNIEnv*`, `jclass`) with no arguments.
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isaacduarte_wellfed_RustHolds_clear<'caller>(
    _env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) {
    crate::alerts::clear_holds();
}

/// `Uplink.set(handle)`, a static native method.
// SAFETY: the JVM resolves `Uplink.set` to this exact name, and the
// signature is a static method's (`JNIEnv*`, `jclass`, `jlong`).
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_isaacduarte_wellfed_Uplink_set<'caller>(
    _env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    handle: jni::sys::jlong,
) {
    // A `net_handle_t` is unsigned; Java has only the signed long.
    crate::relay::set_uplink(u64::from_ne_bytes(handle.to_ne_bytes()));
}

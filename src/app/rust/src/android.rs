//! The JNI hook Kotlin calls with no Dart running.
//!
//! `KeepAliveService` calls it when the owner swipes the app's task away:
//! the engine's holds on catch-ups go with the engine, since none of its
//! runs is left to let go. A run's thread that still lives (an update
//! pushing chunks or waiting out the restart, an install's listener) keeps
//! its own hold until it ends.

use jni::EnvUnowned;
use jni::objects::JClass;

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

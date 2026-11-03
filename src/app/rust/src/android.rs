//! The JNI hook `MainActivity` calls once at startup, before Dart runs:
//! it gives the TLS certificate verifier the app's context, so an `https`
//! server is checked against Android's own trust store, a CA the owner
//! installed on the phone included. reqwest's rustls asks that verifier,
//! which panics on the first check if it was never given the context.
//!
//! The Kotlin half of the verifier ships in the APK from the
//! `org.rustls:rustls-platform-verifier` Maven package, at the version the
//! lockfile pins for `rustls-platform-verifier-android`.

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

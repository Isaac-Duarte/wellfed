package io.github.isaacduarte.wellfed

import android.content.Context

// The Rust bridge checks an https server's certificate with Android's own
// trust store, through rustls-platform-verifier, which must be handed the
// app's context before its first check. Loading the library here is the
// same one Dart opens, so both see one verifier.
object TlsVerifier {
    init {
        System.loadLibrary("wellfed_app_bridge")
    }

    @JvmStatic external fun start(context: Context)
}

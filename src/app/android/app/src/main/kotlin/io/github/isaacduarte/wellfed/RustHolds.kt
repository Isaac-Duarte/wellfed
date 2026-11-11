package io.github.isaacduarte.wellfed

// The Rust bridge keeps catch-ups off while a run (an install, a restore, an
// update) holds them, and each run lets go when it ends. A run that dies with
// the app's task never does, so the keep-alive service clears them all here
// when the task is swiped away, rather than leave catch-ups off for the two
// hours a hold takes to lapse.
object RustHolds {
    init {
        System.loadLibrary("wellfed_app_bridge")
    }

    @JvmStatic external fun clear()
}

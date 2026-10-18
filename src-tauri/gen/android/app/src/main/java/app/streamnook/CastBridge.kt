package app.streamnook

/**
 * JNI door from the Cast SDK's callbacks into the Rust cast session
 * (`cast_plugin.rs`, `services/cast_session.rs`).
 *
 * A plain `object` for the same reason as NotifyBridge: its members compile to
 * instance methods on this class, so the Rust symbol is exactly
 * `Java_app_streamnook_CastBridge_onEvent`. A companion object would put it on
 * `CastBridge$Companion` and the binding would fail only at call time.
 */
object CastBridge {
  init {
    System.loadLibrary("streamnook_lib")
  }

  /**
   * One event as JSON: `{"kind":"available"|"session"|"media"|"load_failed", ...}`.
   * Rust only parses and hands it to its runtime, so this is safe on the main thread.
   */
  external fun onEvent(json: String)
}

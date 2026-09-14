# StreamNook keep rules (picked up by the release fileTree("**/*.pro") glob).

# The MainActivity insets bridge is called reflectively from JS via
# addJavascriptInterface; R8 must not strip or rename it.
-keepclassmembers class app.streamnook.MainActivity$InsetsBridge {
    @android.webkit.JavascriptInterface <methods>;
}

# Tauri mobile plugins are instantiated reflectively by PluginManager.
-keep class app.streamnook.TwitchLoginPlugin { *; }

# The JNI symbol Rust exports is derived from this class's fully qualified name,
# so a rename breaks the binding at call time rather than at build time. The
# default androidx rules would likely cover it via -keepclasseswithmembernames,
# but this only fails in release builds, which is the worst place to find out.
-keep class app.streamnook.NotifyBridge {
    native <methods>;
}

# WorkManager instantiates workers reflectively from the class name it persisted
# when the work was enqueued, so a rename fails only after an update, on jobs
# that were already scheduled.
-keep class app.streamnook.NotifyWorker { *; }

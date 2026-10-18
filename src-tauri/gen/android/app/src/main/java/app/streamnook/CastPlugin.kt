package app.streamnook

import android.app.Activity
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

@InvokeArg
class CastLoadArgs {
  lateinit var url: String
  var segmentFormat: String = "ts"
  var audioOnly: Boolean = false
  var title: String = ""
  var subtitle: String = ""
  var custom: String = "{}"
}

@InvokeArg
class CastControlArgs {
  lateinit var action: String
}

@InvokeArg
class TvShowArgs {
  lateinit var url: String
}

/**
 * Rust's handle on [CastRemote] (`cast_plugin.rs` calls these through
 * `run_mobile_plugin`). The page never calls this plugin directly: plugin
 * commands need an ACL grant an app-local plugin does not get, so the page goes
 * through app commands that forward here, the same as TwitchLoginPlugin.
 *
 * Everything hops to the main thread, where the Cast SDK insists on being called.
 */
@TauriPlugin
class CastPlugin(private val activity: Activity) : Plugin(activity) {
  @Command
  fun showPicker(invoke: Invoke) {
    activity.runOnUiThread {
      if (CastRemote.showPicker(activity)) invoke.resolve() else invoke.reject("Casting isn't available on this phone")
    }
  }

  @Command
  fun load(invoke: Invoke) {
    val args = invoke.parseArgs(CastLoadArgs::class.java)
    activity.runOnUiThread {
      val sent = CastRemote.load(args.url, args.segmentFormat, args.audioOnly, args.title, args.subtitle, args.custom)
      if (sent) invoke.resolve() else invoke.reject("No cast session")
    }
  }

  @Command
  fun control(invoke: Invoke) {
    val args = invoke.parseArgs(CastControlArgs::class.java)
    activity.runOnUiThread {
      if (CastRemote.command(args.action)) invoke.resolve() else invoke.reject("No cast session")
    }
  }

  @Command
  fun tvShow(invoke: Invoke) {
    val args = invoke.parseArgs(TvShowArgs::class.java)
    activity.runOnUiThread {
      if (TvScreen.show(args.url)) invoke.resolve() else invoke.reject("No TV to show on")
    }
  }

  @Command
  fun tvHide(invoke: Invoke) {
    activity.runOnUiThread {
      TvScreen.hide()
      invoke.resolve()
    }
  }

  @Command
  fun tvControl(invoke: Invoke) {
    val args = invoke.parseArgs(CastControlArgs::class.java)
    activity.runOnUiThread {
      if (TvScreen.control(args.action)) invoke.resolve() else invoke.reject("Nothing playing on the TV")
    }
  }

  @Command
  fun openMirroring(invoke: Invoke) {
    activity.runOnUiThread {
      invoke.resolve(JSObject().put("opened", CastRemote.openMirroring(activity)))
    }
  }
}

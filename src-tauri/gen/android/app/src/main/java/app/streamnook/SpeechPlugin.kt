package app.streamnook

import android.app.Activity
import android.os.Handler
import android.os.Looper
import android.speech.tts.TextToSpeech
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.Locale

@InvokeArg
class SpeakArgs {
  lateinit var text: String
}

/**
 * Text to speech for automations' Speak action and /tts (`speech_plugin.rs`
 * calls `speak` through `run_mobile_plugin`; the page goes through the app
 * command `speak_android`, as with CastPlugin).
 *
 * It never requests audio focus: a line is spoken over whatever is playing, so
 * the stream keeps its volume and the system mixes. The engine is made on
 * first use and a line waits for it. Everything runs on the main thread, where
 * the TextToSpeech callbacks arrive. The answer is `{ spoken }`: false when the
 * phone has no speech engine, no voice for its language, or the engine does not
 * start within 10 seconds; the page then shows the text instead. A failed
 * engine is tried again after a minute.
 */
@TauriPlugin
class SpeechPlugin(private val activity: Activity) : Plugin(activity) {
  private val main = Handler(Looper.getMainLooper())
  private var tts: TextToSpeech? = null
  private var ready = false
  private var starting = false
  private var failedAt = 0L
  private var counter = 0
  private val waiting = ArrayList<Pair<String, Invoke>>()

  @Command
  fun speak(invoke: Invoke) {
    val args = invoke.parseArgs(SpeakArgs::class.java)
    main.post { enqueue(args.text, invoke) }
  }

  private fun enqueue(text: String, invoke: Invoke) {
    if (failedAt != 0L) {
      if (System.currentTimeMillis() - failedAt < RETRY_MS) return answer(invoke, false)
      // Time to try the engine again.
      failedAt = 0L
      starting = false
    }
    if (ready) {
      say(text, invoke)
    } else {
      waiting.add(Pair(text, invoke))
      start()
    }
  }

  private fun start() {
    if (starting) return
    starting = true
    tts = TextToSpeech(activity.applicationContext) { status -> main.post { onInit(status) } }
    // An engine that never answers fails the waiting lines instead of hanging them.
    main.postDelayed({ if (!ready && failedAt == 0L) fail() }, INIT_TIMEOUT_MS)
  }

  private fun onInit(status: Int) {
    if (failedAt != 0L) return
    val engine = tts
    if (status != TextToSpeech.SUCCESS || engine == null) return fail()
    val language = engine.setLanguage(Locale.getDefault())
    if (language == TextToSpeech.LANG_MISSING_DATA || language == TextToSpeech.LANG_NOT_SUPPORTED) return fail()
    ready = true
    val lines = ArrayList(waiting)
    waiting.clear()
    for ((text, invoke) in lines) say(text, invoke)
  }

  private fun say(text: String, invoke: Invoke) {
    val engine = tts ?: return answer(invoke, false)
    // QUEUE_ADD: lines never cut each other off. No params, so no audio focus is requested.
    val code = engine.speak(text, TextToSpeech.QUEUE_ADD, null, "sn-speak-${counter++}")
    answer(invoke, code == TextToSpeech.SUCCESS)
  }

  private fun fail() {
    failedAt = System.currentTimeMillis()
    ready = false
    tts?.shutdown()
    tts = null
    val lines = ArrayList(waiting)
    waiting.clear()
    for ((_, invoke) in lines) answer(invoke, false)
  }

  private fun answer(invoke: Invoke, spoken: Boolean) {
    invoke.resolve(JSObject().put("spoken", spoken))
  }

  companion object {
    private const val INIT_TIMEOUT_MS = 10_000L
    private const val RETRY_MS = 60_000L
  }
}

package app.streamnook

import android.content.Context
import com.google.android.gms.cast.CastMediaControlIntent
import com.google.android.gms.cast.framework.CastOptions
import com.google.android.gms.cast.framework.OptionsProvider
import com.google.android.gms.cast.framework.SessionProvider
import com.google.android.gms.cast.framework.media.CastMediaOptions

/**
 * Google Cast configuration, read by the SDK through the manifest's
 * OPTIONS_PROVIDER_CLASS_NAME entry.
 *
 * RECEIVER. Google's Default Media Receiver: free, unregistered, and it plays
 * HLS that it fetches itself. The registered StreamNook receiver (app id
 * 9A37B2A5, https://streamnook.app/cast/) replaces it once that page exists;
 * it is what will run the ad filter on the TV.
 *
 * NO SDK NOTIFICATION, NO SDK MEDIA SESSION. MediaPlaybackService already owns
 * the phone's media card and session, and while casting its buttons drive the
 * TV (CastRemote). Two sessions would show two cards and fight over the lock
 * screen, which the Output Switcher guide lists as a known issue. The volume
 * keys the SDK would have handled through its session are wired in
 * MainActivity.dispatchKeyEvent instead.
 */
class CastOptionsProvider : OptionsProvider {
  override fun getCastOptions(context: Context): CastOptions {
    val media = CastMediaOptions.Builder()
      .setNotificationOptions(null)
      .setMediaSessionEnabled(false)
      .build()
    return CastOptions.Builder()
      .setReceiverApplicationId(CastMediaControlIntent.DEFAULT_MEDIA_RECEIVER_APPLICATION_ID)
      .setCastMediaOptions(media)
      .build()
  }

  override fun getAdditionalSessionProviders(context: Context): List<SessionProvider>? = null
}

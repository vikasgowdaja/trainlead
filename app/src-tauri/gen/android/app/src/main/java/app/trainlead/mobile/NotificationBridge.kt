package app.trainlead.mobile

import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.os.Build
import android.provider.Settings
import android.service.notification.NotificationListenerService
import android.webkit.JavascriptInterface
import androidx.core.app.NotificationManagerCompat

/** Exposed to the app's own WebView page as `window.TrainLeadAndroid`. */
class NotificationBridge(private val activity: Activity) {
  private val listener = ComponentName(activity, WhatsAppListener::class.java)

  @JavascriptInterface
  fun isAccessGranted(): Boolean =
    NotificationManagerCompat.getEnabledListenerPackages(activity).contains(activity.packageName)

  @JavascriptInterface
  fun openAccessSettings() {
    val intent = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      Intent(Settings.ACTION_NOTIFICATION_LISTENER_DETAIL_SETTINGS)
        .putExtra(Settings.EXTRA_NOTIFICATION_LISTENER_COMPONENT_NAME, listener.flattenToString())
    } else {
      Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
    }
    activity.runOnUiThread {
      runCatching { activity.startActivity(intent) }
        .onFailure { activity.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)) }
    }
  }

  @JavascriptInterface
  fun openWhatsApp(): Boolean {
    val launch = NotificationQueue.packages.firstNotNullOfOrNull {
      activity.packageManager.getLaunchIntentForPackage(it)
    } ?: return false
    activity.runOnUiThread { activity.startActivity(launch) }
    return true
  }

  /** Re-reads the notification shade, then returns up to [limit] queued messages as JSON. */
  @JavascriptInterface
  fun peek(limit: Int): String {
    if (isAccessGranted()) {
      val running = WhatsAppListener.instance
      if (running == null) NotificationListenerService.requestRebind(listener) else running.scan()
    }
    return NotificationQueue.peek(activity, limit.coerceIn(1, 200))
  }

  @JavascriptInterface
  fun ack(count: Int) = NotificationQueue.ack(activity, count)

  @JavascriptInterface
  fun pending(): Int = NotificationQueue.pending(activity)
}

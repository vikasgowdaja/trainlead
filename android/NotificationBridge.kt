package app.trainlead
// Reads WhatsApp notifications and forwards them. Muted groups and long messages may be missed.
import android.app.Notification
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import org.json.JSONArray
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import kotlin.concurrent.thread

class NotificationBridge : NotificationListenerService() {
    private val api = "http://YOUR-SERVER:8000/api/ingest/messages"
    private val key = "YOUR-TL_KEY"
    override fun onNotificationPosted(sbn: StatusBarNotification) {
        if (sbn.packageName != "com.whatsapp" && sbn.packageName != "com.whatsapp.w4b") return
        if ((sbn.notification.flags and Notification.FLAG_GROUP_SUMMARY) != 0) return
        val e = sbn.notification.extras
        val title = e.getString(Notification.EXTRA_TITLE) ?: return
        val text = (e.getCharSequence(Notification.EXTRA_BIG_TEXT) ?: e.getCharSequence(Notification.EXTRA_TEXT))?.toString() ?: return
        val i = text.indexOf(": ")
        val msg = JSONObject().put("sender", if (i > 0) text.substring(0, i) else JSONObject.NULL).put("text", if (i > 0) text.substring(i + 2) else text)
        val body = JSONObject().put("source", title).put("messages", JSONArray().put(msg)).toString()
        thread {
            try {
                (URL(api).openConnection() as HttpURLConnection).run {
                    requestMethod = "POST"; doOutput = true
                    setRequestProperty("content-type", "application/json"); setRequestProperty("x-key", key)
                    outputStream.use { it.write(body.toByteArray()) }; responseCode; disconnect()
                }
            } catch (_: Exception) {}
        }
    }
}

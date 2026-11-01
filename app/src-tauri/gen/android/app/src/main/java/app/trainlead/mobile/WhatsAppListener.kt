package app.trainlead.mobile

import android.app.Notification
import android.content.Context
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import androidx.core.app.NotificationCompat
import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.security.MessageDigest

/** Copies WhatsApp chat notifications into a private queue that the app imports on demand. */
class WhatsAppListener : NotificationListenerService() {
  override fun onListenerConnected() {
    instance = this
    scan()
  }

  override fun onListenerDisconnected() {
    if (instance === this) instance = null
  }

  override fun onNotificationPosted(sbn: StatusBarNotification) {
    NotificationQueue.record(this, sbn)
  }

  /** Re-reads notifications still in the shade, e.g. ones posted before access was granted. */
  fun scan() {
    try {
      activeNotifications?.forEach { NotificationQueue.record(this, it) }
    } catch (_: SecurityException) {
      // Access was revoked; the bridge reports that state.
    }
  }

  companion object {
    @Volatile var instance: WhatsAppListener? = null
  }
}

object NotificationQueue {
  val packages = setOf("com.whatsapp", "com.whatsapp.w4b")
  private const val QUEUE = "whatsapp-queue.jsonl"
  private const val SEEN = "whatsapp-seen.txt"
  private const val MAX_QUEUE = 5000
  private const val MAX_SEEN = 3000
  private val countSuffix = Regex("""\s*\(\d+ (new )?messages?\)$""")
  private val chatter = Regex("""^(\d+ new messages?|checking for new messages|.*messages? from \d+ chats?)$""", RegexOption.IGNORE_CASE)
  private var seen: LinkedHashSet<String>? = null

  @Synchronized
  fun record(context: Context, sbn: StatusBarNotification) {
    if (sbn.packageName !in packages) return
    val notification = sbn.notification
    val skipped = Notification.FLAG_GROUP_SUMMARY or Notification.FLAG_ONGOING_EVENT
    if (notification.flags and skipped != 0) return
    val known = seen(context)
    // WhatsApp re-posts the whole conversation with each new message, so drop ones already queued.
    val fresh = parse(notification, sbn.postTime).filter { known.add(key(it)) }
    if (fresh.isEmpty()) return
    while (known.size > MAX_SEEN) known.remove(known.first())
    File(context.filesDir, SEEN).writeText(known.joinToString("\n"))
    val queue = File(context.filesDir, QUEUE)
    if (queue.exists() && lines(queue).size >= MAX_QUEUE) return
    queue.appendText(fresh.joinToString("") { "$it\n" })
  }

  /** Oldest queued messages first; call [ack] after they are saved so a failed import loses nothing. */
  @Synchronized
  fun peek(context: Context, limit: Int): String {
    val array = JSONArray()
    lines(File(context.filesDir, QUEUE)).take(limit).forEach { line ->
      runCatching { array.put(JSONObject(line)) }
    }
    return array.toString()
  }

  @Synchronized
  fun ack(context: Context, count: Int) {
    val queue = File(context.filesDir, QUEUE)
    val rest = lines(queue).drop(count.coerceAtLeast(0))
    if (rest.isEmpty()) queue.delete() else queue.writeText(rest.joinToString("") { "$it\n" })
  }

  @Synchronized
  fun pending(context: Context): Int = lines(File(context.filesDir, QUEUE)).size

  private fun lines(file: File): List<String> =
    if (file.exists()) file.readLines().filter { it.isNotBlank() } else emptyList()

  private fun seen(context: Context): LinkedHashSet<String> =
    seen ?: LinkedHashSet<String>().also { set ->
      val file = File(context.filesDir, SEEN)
      if (file.exists()) set.addAll(file.readLines().filter { it.isNotBlank() })
      seen = set
    }

  private fun key(entry: JSONObject): String {
    val raw = listOf("group", "sender", "timestamp", "text").joinToString("\u0000") { entry.optString(it) }
    return MessageDigest.getInstance("SHA-256").digest(raw.toByteArray())
      .take(12).joinToString("") { "%02x".format(it) }
  }

  private fun entry(group: String, sender: String?, timestamp: Long, text: String): JSONObject =
    JSONObject()
      .put("group", group.replace(countSuffix, "").trim())
      .put("sender", sender ?: JSONObject.NULL)
      .put("timestamp", timestamp)
      .put("text", text)

  private fun parse(notification: Notification, posted: Long): List<JSONObject> {
    val extras = notification.extras
    val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString()?.trim()
    val style = NotificationCompat.MessagingStyle.extractMessagingStyleFromNotification(notification)
    if (style != null && style.messages.isNotEmpty()) {
      val conversation = style.conversationTitle?.toString()?.trim()?.takeIf { it.isNotEmpty() }
      return style.messages.mapNotNull { message ->
        val text = message.text?.toString()?.trim().orEmpty()
        val sender = message.person?.name?.toString()?.trim()?.takeIf { it.isNotEmpty() }
        // A null person is the phone owner's own reply.
        if (text.isEmpty() || sender == null || chatter.matches(text)) return@mapNotNull null
        val group = conversation ?: "WhatsApp: ${title ?: sender}"
        entry(group, sender, if (message.timestamp > 0) message.timestamp else posted, text)
      }
    }
    val text = (extras.getCharSequence(Notification.EXTRA_BIG_TEXT)
      ?: extras.getCharSequence(Notification.EXTRA_TEXT))?.toString()?.trim()
    if (title.isNullOrEmpty() || text.isNullOrEmpty() || title == "WhatsApp" || chatter.matches(text)) return emptyList()
    // Plain group notifications read "Sender: text" with the group as the title.
    val split = text.indexOf(": ")
    return if (split in 1..80) {
      listOf(entry(title, text.substring(0, split), posted, text.substring(split + 2)))
    } else {
      listOf(entry("WhatsApp: $title", title, posted, text))
    }
  }
}

package io.github.yashwanthkamireddi.skykeep.sync

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import io.github.yashwanthkamireddi.skykeep.MainActivity
import io.github.yashwanthkamireddi.skykeep.R

/**
 * Apple ended the session (it wants the password, or a code): the engine can't take it up again
 * by itself, so every background sync after it would fail the same way, quietly, for good. Instead
 * the account's background work stops, and one calm notification says what to do; it opens the
 * app at sign-in. Signing in again clears both and the work starts again ([ensureAccount]).
 */
object SessionEnded {
  private const val CHANNEL = "account"
  private const val PREFS = "session-ended"
  private const val NOTICE = 7201

  /** The extra on the notification's intent: the app opens at sign-in. */
  const val SIGN_IN = "io.github.yashwanthkamireddi.skykeep.SIGN_IN"

  private fun prefs(context: Context) = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

  /** Whether Apple ended `tag`'s session and nobody has signed in since: no pass runs for it. */
  fun ended(context: Context, tag: String): Boolean = prefs(context).getBoolean(tag, false)

  /**
   * Apple ended `tag`'s session: its background work stops and the person is told, once. Asked
   * again (another service's pass found the same), nothing more is said.
   */
  fun mark(context: Context, tag: String) {
    val first = !prefs(context).getBoolean(tag, false)
    prefs(context).edit().putBoolean(tag, true).commit()
    Work.cancelAll(context, tag)
    if (first) tell(context)
  }

  /** Signed in again: the work goes on, and the notification is taken down. */
  fun clear(context: Context) {
    prefs(context).edit().clear().commit()
    NotificationManagerCompat.from(context).cancel(NOTICE)
  }

  private fun tell(context: Context) {
    val manager = NotificationManagerCompat.from(context)
    if (!manager.areNotificationsEnabled()) return
    val system = context.getSystemService(NotificationManager::class.java)
    if (system.getNotificationChannel(CHANNEL) == null) {
      system.createNotificationChannel(
        NotificationChannel(CHANNEL, "Your account", NotificationManager.IMPORTANCE_DEFAULT).apply {
          description = "When Skykeep needs you to sign in again to keep syncing"
          setSound(null, null)
        },
      )
    }
    val open = PendingIntent.getActivity(
      context, NOTICE,
      Intent(context, MainActivity::class.java).putExtra(SIGN_IN, true)
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP),
      PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
    )
    val note = NotificationCompat.Builder(context, CHANNEL)
      .setSmallIcon(R.drawable.ic_sync)
      .setContentTitle("Sign in to Skykeep again to keep syncing")
      .setContentText("Everything on this phone is still here.")
      .setCategory(NotificationCompat.CATEGORY_STATUS)
      .setContentIntent(open)
      .setAutoCancel(true)
      .setOnlyAlertOnce(true)
      .build()
    runCatching { manager.notify(NOTICE, note) }
  }
}

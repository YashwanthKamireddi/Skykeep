package io.github.yashwanthkamireddi.skykeep.sync

import android.app.AlarmManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.Bundle
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import io.github.yashwanthkamireddi.skykeep.MainActivity
import io.github.yashwanthkamireddi.skykeep.R

/**
 * A reminder's alert at its time, as an iPhone rings one: set from the library after every sync
 * and every change made here, for the next reminder due; when it fires, every reminder due by
 * then is said once, each with Completed, and the next alert set. Reset after a restart (Android
 * forgets alarms). An all-day reminder is said at 9 AM, as Apple's are.
 */
object ReminderAlerts {
  private const val CHANNEL = "reminders"
  private const val PREFS = "reminder-alerts"
  private const val ACTION_FIRE = "io.github.yashwanthkamireddi.skykeep.REMINDER_DUE"
  private const val ACTION_DONE = "io.github.yashwanthkamireddi.skykeep.REMINDER_DONE"
  private const val ALL_DAY_AT_SECONDS = 9 * 3600L

  /** When a reminder is said: its time, or 9 AM on its day for an all-day one. */
  private fun ringsAt(r: uniffi.quince_mobile.RItem): Long? = r.dueAt?.let { if (r.allDay) it + ALL_DAY_AT_SECONDS else it }

  /** Each reminder said once per due time: `guid@due` kept, so a sync never says it again. */
  private fun said(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

  private fun fireIntent(context: Context) = PendingIntent.getBroadcast(
    context, 0, Intent(context, ReminderAlertReceiver::class.java).setAction(ACTION_FIRE),
    PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
  )

  /**
   * Signed out: the next alert cancelled and every reminder notification taken down, so one
   * account's reminders are never shown after it leaves the phone. Other notifications stay.
   */
  fun stop(context: Context) {
    context.getSystemService(AlarmManager::class.java).cancel(fireIntent(context))
    val manager = context.getSystemService(NotificationManager::class.java)
    for (n in manager.activeNotifications) if (n.notification.channelId == CHANNEL) manager.cancel(n.tag, n.id)
    said(context).edit().clear().commit()
  }

  /** The next alert set (or none), after saying whatever is due now. Signed out, none. */
  fun schedule(context: Context) {
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
    if (!engine.hasSession()) return stop(context)
    val now = System.currentTimeMillis() / 1000
    val soon = runCatching { engine.remindersDueBetween(now - 86_400, now + 14 * 86_400) }.getOrNull() ?: return
    val open = soon.filter { it.completedAt == null }
    sayDue(context, open.filter { (ringsAt(it) ?: Long.MAX_VALUE) <= now })
    val next = open.mapNotNull(::ringsAt).filter { it > now }.minOrNull()
    val alarms = context.getSystemService(AlarmManager::class.java)
    val fire = fireIntent(context)
    alarms.cancel(fire)
    if (next == null) return
    val at = next * 1000
    // On the minute where Android allows it; within a few minutes where it doesn't.
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S || alarms.canScheduleExactAlarms()) {
      alarms.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, fire)
    } else {
      alarms.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, fire)
    }
  }

  private fun sayDue(context: Context, due: List<uniffi.quince_mobile.RItem>) {
    if (due.isEmpty()) return
    val manager = NotificationManagerCompat.from(context)
    if (!manager.areNotificationsEnabled()) return
    channel(context)
    val prefs = said(context)
    val edit = prefs.edit()
    for (r in due) {
      val key = "${r.guid}@${r.dueAt}"
      if (prefs.getBoolean(key, false)) continue
      val id = r.guid.hashCode()
      val open = PendingIntent.getActivity(
        context, id, Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
      )
      val done = PendingIntent.getBroadcast(
        context, id, Intent(context, ReminderAlertReceiver::class.java).setAction(ACTION_DONE).putExtra("guid", r.guid),
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
      )
      val note = NotificationCompat.Builder(context, CHANNEL)
        .setSmallIcon(R.drawable.ic_reminders)
        .setContentTitle(r.title.ifBlank { "Reminder" })
        .apply { r.notes?.takeIf { it.isNotBlank() }?.let { setContentText(it) } }
        .setCategory(NotificationCompat.CATEGORY_REMINDER)
        .setPriority(NotificationCompat.PRIORITY_HIGH)
        .setContentIntent(open)
        .setAutoCancel(true)
        .addAction(0, "Completed", done)
        .build()
      runCatching { manager.notify(id, note) }
      edit.putBoolean(key, true)
    }
    edit.apply()
  }

  private fun channel(context: Context) {
    val manager = context.getSystemService(NotificationManager::class.java)
    if (manager.getNotificationChannel(CHANNEL) == null) {
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL, "Reminders", NotificationManager.IMPORTANCE_HIGH).apply {
          description = "Your iCloud reminders, at the time they're due"
        },
      )
    }
  }

  internal fun onFire(context: Context) = schedule(context)

  /** Completed from the alert: ticked in the library at once, sent with a Reminders sync. */
  internal fun onDone(context: Context, guid: String) {
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
    // A tick that can't be recorded leaves the alert up, so it can be pressed again or opened,
    // and says why in the log; it is never dropped quietly.
    val ticked = runCatching { engine.reminderTick(guid, true) }
    ticked.exceptionOrNull()?.let {
      android.util.Log.w("Skykeep", "Completed from an alert wasn't recorded: ${(it as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: it}")
      return
    }
    NotificationManagerCompat.from(context).cancel(guid.hashCode())
    val account = io.github.yashwanthkamireddi.skykeep.ui.phoneAccount(context)
    if (account == null) android.util.Log.w("Skykeep", "Completed from an alert: no account to sync")
    account?.let {
      ContentResolver.requestSync(it, REMINDERS_AUTHORITY, Bundle().apply {
        putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
        putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
      })
    }
    schedule(context)
  }

  internal const val FIRE = ACTION_FIRE
  internal const val DONE = ACTION_DONE
}

/** The alert's time, Completed pressed on one, or the phone restarted (alarms are forgotten). */
class ReminderAlertReceiver : BroadcastReceiver() {
  override fun onReceive(context: Context, intent: Intent) {
    val pending = goAsync()
    Thread {
      try {
        when (intent.action) {
          ReminderAlerts.DONE -> intent.getStringExtra("guid")?.let { ReminderAlerts.onDone(context, it) }
          else -> ReminderAlerts.onFire(context)
        }
      } finally {
        pending.finish()
      }
    }.start()
  }
}

package io.github.yashwanthkamireddi.skykeep.sync

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.net.Uri

/**
 * Reminders' place in Android's sync: the account syncs Contacts, Calendar and Reminders, on the
 * one schedule Settings sets, each shown in the account's own sync settings. Android has no
 * Reminders store of its own, so this names one: the reminders live in the engine's library
 * (quince-sync, the desktop's own code), and this provider holds nothing.
 */
val REMINDERS_AUTHORITY = "${io.github.yashwanthkamireddi.skykeep.BuildConfig.APPLICATION_ID}.reminders"

/** The authority the sync is listed under. It stores nothing; every call answers empty. */
class RemindersProvider : ContentProvider() {
  override fun onCreate() = true
  override fun query(u: Uri, p: Array<out String>?, s: String?, a: Array<out String>?, o: String?): Cursor? = null
  override fun getType(u: Uri): String? = null
  override fun insert(u: Uri, v: ContentValues?): Uri? = null
  override fun delete(u: Uri, s: String?, a: Array<out String>?) = 0
  override fun update(u: Uri, v: ContentValues?, s: String?, a: Array<out String>?) = 0
}

/**
 * Reminders: the changes made here sent to Apple, oldest first, then Apple's Reminders read and
 * laid under whatever is still on its way; then the next alerts set, whatever happened.
 */
internal fun remindersPass(context: Context, engine: uniffi.quince_mobile.AccountInterface, authority: String): Pass = try {
  val done = engine.remindersSync()
  val said = buildList {
    add("${done.reminders} open in ${done.lists} lists")
    if (done.waiting > 0u) add("${done.waiting} changes to send again")
    if (done.refused.isNotEmpty()) add("${done.refused.size} not taken by iCloud: ${done.refused.first()}")
  }.joinToString(" · ")
  Outcome.record(context, authority, true, said, paused = done.paused)
  // Waiting on an approval is not tried again on a timer: it goes once the person approves.
  if (done.waiting > 0u && !done.paused) Pass.Again else Pass.Done
} catch (x: Exception) {
  if (!x.fromEngine()) throw x
  Outcome.record(context, authority, false, x.reason("Reminders couldn't be read."))
  Pass.Again
} finally {
  ReminderAlerts.schedule(context)
}

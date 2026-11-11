package io.github.yashwanthkamireddi.skykeep.sync

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.net.Uri

/**
 * Notes' place in Android's sync, as Reminders' is (RemindersSync.kt): the notes live in the
 * engine's library (quince-sync, the desktop's own code), and this provider holds nothing.
 */
val NOTES_AUTHORITY = "${io.github.yashwanthkamireddi.skykeep.BuildConfig.APPLICATION_ID}.notes"

/** The authority the sync is listed under. It stores nothing; every call answers empty. */
class NotesProvider : ContentProvider() {
  override fun onCreate() = true
  override fun query(u: Uri, p: Array<out String>?, s: String?, a: Array<out String>?, o: String?): Cursor? = null
  override fun getType(u: Uri): String? = null
  override fun insert(u: Uri, v: ContentValues?): Uri? = null
  override fun delete(u: Uri, s: String?, a: Array<out String>?) = 0
  override fun update(u: Uri, v: ContentValues?, s: String?, a: Array<out String>?) = 0
}

/** Notes: what was changed here sent, oldest first, then Apple's Notes read. */
internal fun notesPass(context: Context, engine: uniffi.quince_mobile.AccountInterface, authority: String): Pass = try {
  val done = engine.notesSync()
  val said = buildList {
    add("${done.notes} notes in ${done.folders} folders")
    if (done.waiting > 0u) add("${done.waiting} changes to send again")
    if (done.refused.isNotEmpty()) add("${done.refused.size} not taken by iCloud: ${done.refused.first()}")
  }.joinToString(" · ")
  Outcome.record(context, authority, true, said, paused = done.paused)
  // Waiting on an approval is not tried again on a timer: it goes once the person approves.
  if (done.waiting > 0u && !done.paused) Pass.Again else Pass.Done
} catch (x: Exception) {
  if (!x.fromEngine()) throw x
  Outcome.record(context, authority, false, x.reason("Notes couldn't be read."))
  Pass.Again
}

package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.app.Service
import android.content.AbstractThreadedSyncAdapter
import android.content.ContentProvider
import android.content.ContentProviderClient
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.SyncResult
import android.database.Cursor
import android.net.Uri
import android.os.Bundle
import android.os.IBinder

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

/** Android asking for Notes: what was changed here sent, oldest first, then Apple's Notes read. */
private class NotesAdapter(context: Context) : AbstractThreadedSyncAdapter(context, true) {
  override fun onPerformSync(a: Account, e: Bundle, authority: String, p: ContentProviderClient, r: SyncResult) {
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
    try {
      val done = engine.notesSync()
      val said = buildList {
        add("${done.notes} notes in ${done.folders} folders")
        if (done.waiting > 0u) add("${done.waiting} changes to send again")
        if (done.refused.isNotEmpty()) add("${done.refused.size} not taken by iCloud: ${done.refused.first()}")
      }.joinToString(" · ")
      if (done.waiting > 0u) r.stats.numIoExceptions++
      Outcome.record(context, authority, true, said)
    } catch (x: uniffi.quince_mobile.EngineException) {
      r.stats.numIoExceptions++
      Outcome.record(context, authority, false, (x as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "Notes couldn't be read.")
    }
  }
}

class NotesSyncService : Service() {
  private val adapter by lazy { NotesAdapter(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

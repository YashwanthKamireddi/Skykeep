package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.app.Service
import android.content.AbstractThreadedSyncAdapter
import android.content.ContentProvider
import android.content.ContentProviderClient
import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.SyncResult
import android.database.Cursor
import android.net.Uri
import android.os.Bundle
import android.os.IBinder

/**
 * Photos' place in Android's sync, as Notes' is (NotesSync.kt): the library is read from Apple a
 * few pages a pass into the engine's library; the pictures are fetched when shown.
 */
val PHOTOS_AUTHORITY = "${io.github.yashwanthkamireddi.skykeep.BuildConfig.APPLICATION_ID}.photos"

/** The authority the sync is listed under. It stores nothing; every call answers empty. */
class PhotosProvider : ContentProvider() {
  override fun onCreate() = true
  override fun query(u: Uri, p: Array<out String>?, s: String?, a: Array<out String>?, o: String?): Cursor? = null
  override fun getType(u: Uri): String? = null
  override fun insert(u: Uri, v: ContentValues?): Uri? = null
  override fun delete(u: Uri, s: String?, a: Array<out String>?) = 0
  override fun update(u: Uri, v: ContentValues?, s: String?, a: Array<out String>?) = 0
}

/** Android asking for Photos: a pass of Apple's library; a large one is read over several. */
private class PhotosAdapter(context: Context) : AbstractThreadedSyncAdapter(context, true) {
  override fun onPerformSync(a: Account, e: Bundle, authority: String, p: ContentProviderClient, r: SyncResult) {
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
    try {
      val done = engine.photosSync()
      val said = when {
        done.preparing -> "Apple is still preparing your library"
        done.more -> "${done.photos} photos so far, more on the next sync"
        else -> "${done.photos} photos"
      }
      Outcome.record(context, authority, true, said)
      // New photos taken here, when backup is on: sent after the library is read.
      PhotoBackup.run(context)
      // A large library is read over several passes: the next is asked for at once.
      if (done.more) ContentResolver.requestSync(a, authority, Bundle())
    } catch (x: uniffi.quince_mobile.EngineException) {
      r.stats.numIoExceptions++
      Outcome.record(context, authority, false, (x as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "Photos couldn't be read.")
    }
  }
}

class PhotosSyncService : Service() {
  private val adapter by lazy { PhotosAdapter(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

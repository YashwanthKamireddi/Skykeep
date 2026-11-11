package io.github.yashwanthkamireddi.skykeep.sync

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.database.Cursor
import android.net.Uri

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

/**
 * Photos: the favourites, hides and deletes made here sent to Apple, oldest first, then a pass
 * of Apple's library laid under whatever is still on its way; a large one is read over several,
 * the next asked for at once ([Pass.More]). New photos taken here are backed up by their own work
 * ([PhotoBackupWorker]), asked for once the library is read.
 */
internal fun photosPass(context: Context, engine: uniffi.quince_mobile.AccountInterface, authority: String): Pass = try {
  val done = engine.photosSync()
  val said = buildList {
    add(when {
      done.preparing -> "Apple is still preparing your library"
      done.more -> "${done.photos} photos so far, more on the next sync"
      else -> "${done.photos} photos"
    })
    if (done.waiting > 0u) add("${done.waiting} changes to send again")
    if (done.refused.isNotEmpty()) add("${done.refused.size} not taken by iCloud: ${done.refused.first()}")
  }.joinToString(" · ")
  Outcome.record(context, authority, true, said, paused = done.paused)
  if (!done.paused) kotlinx.coroutines.runBlocking { PhotoBackup.now(context) }
  when {
    done.more -> Pass.More
    // Changes still waiting (Apple busy or unreachable): again, with the work's backoff.
    done.waiting > 0u && !done.paused -> Pass.Again
    else -> Pass.Done
  }
} catch (x: Exception) {
  if (!x.fromEngine()) throw x
  Outcome.record(context, authority, false, x.reason("Photos couldn't be read."))
  Pass.Again
}

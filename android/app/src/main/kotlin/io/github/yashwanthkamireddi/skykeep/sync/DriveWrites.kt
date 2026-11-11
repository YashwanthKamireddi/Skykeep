package io.github.yashwanthkamireddi.skykeep.sync

import android.content.Context
import android.provider.DocumentsContract
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import io.github.yashwanthkamireddi.skykeep.Engine
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.WriteSent

/**
 * Files saved into iCloud Drive by other apps, on their way to Apple. What an app writes is held
 * in the engine's journal from the moment it opens the file (the desktop folder's own,
 * `quince_vfs::writes`, kept with the account's library in the app's private files, never the
 * cache): the bytes, and the version of Apple's file they were opened from. This work sends what
 * is held once the app has closed it, whenever there is a network, again with backoff until it
 * has gone, and again after the process or the phone restarts. A save is never lost, and never
 * written over another device's change: that goes beside it as a conflicted copy.
 */
object DriveWrites {
  /** The send of everything held, as soon as there is a network; once more after one running. */
  suspend fun send(context: Context) {
    val account = currentAccount(context) ?: return
    val tag = Work.tagOf(context, account) ?: return
    val request = OneTimeWorkRequestBuilder<DriveSendWorker>()
      .setInputData(workDataOf("tag" to tag))
      .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
      .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
      .addTag(Work.accountTag(tag))
      .build()
    Work.enqueueOnceMore(context, Work.name("drive-send", tag), request)
  }

  /**
   * What an earlier run held and had not sent, sent: an app that kept the file open when this
   * process ended never told it it closed. Asked once per process.
   */
  @Volatile private var resumed = false

  fun resume(context: Context) {
    if (resumed) return
    resumed = true
    val app = context.applicationContext
    Work.scope.launch {
      val waiting = runCatching { Engine.account(app).driveWritesWaiting() }.getOrDefault(emptyList())
      if (waiting.isNotEmpty()) send(app)
    }
  }
}

/** Every file held for the signed-in account sent, oldest key first. */
class DriveSendWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
  override suspend fun doWork(): Result = withContext(Dispatchers.IO) {
    val engine = Engine.account(applicationContext)
    // Held for another account (signed out since): its journal waits with its library.
    if (inputData.getString("tag") != engine.accountTag()) return@withContext Result.success()
    var failed = false
    var again = false
    for (id in runCatching { engine.driveWritesWaiting() }.getOrDefault(emptyList())) {
      val item = runCatching { engine.driveItem(id) }.getOrNull()
      val copy = item?.let { DriveFiles.copyOf(applicationContext, it) }
      try {
        when (val sent = engine.driveWriteSend(id, copy?.path)) {
          is WriteSent.Sent -> {
            // The copy is Apple's version now: never fetched again for being older.
            if (sent.settled) copy?.setLastModified(System.currentTimeMillis()) else again = true
            if (sent.conflicted) android.util.Log.w("Skykeep", "A file changed on another device first; this phone's went beside it")
            notifyFolder(sent.item.parentId ?: item?.parentId)
          }
          // Still open in the app writing it: sent once it closes it (DriveProvider asks).
          WriteSent.StillOpen, WriteSent.NotHeld -> Unit
        }
      } catch (e: Exception) {
        if (!e.fromEngine()) throw e
        android.util.Log.w("Skykeep", "A file saved in iCloud Drive wasn't sent yet: ${e.reason("try again")}")
        failed = true
      }
    }
    when {
      failed -> Result.retry()
      // Written again while it went, so still held: sent next. (One still open is sent when
      // its app closes it.)
      again -> { DriveWrites.send(applicationContext); Result.success() }
      else -> Result.success()
    }
  }

  /** The Files app told a folder changed, as it names it ("root:<tag>" for the top). */
  private fun notifyFolder(parent: String?) {
    parent ?: return
    val engine = Engine.account(applicationContext)
    val shown = if (parent == engine.driveRoot()) "root:${engine.accountTag()}" else parent
    applicationContext.contentResolver.notifyChange(
      DocumentsContract.buildChildDocumentsUri("${applicationContext.packageName}.drive", shown), null, false,
    )
  }
}

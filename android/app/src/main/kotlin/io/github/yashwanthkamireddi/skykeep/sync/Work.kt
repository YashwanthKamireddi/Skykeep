package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.accounts.AccountManager
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.ServiceInfo
import androidx.core.app.NotificationCompat
import androidx.work.ExistingWorkPolicy
import androidx.work.ForegroundInfo
import androidx.work.OneTimeWorkRequest
import androidx.work.WorkInfo
import androidx.work.WorkManager
import io.github.yashwanthkamireddi.skykeep.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * The background work's shared rules, as WorkManager runs it: every piece of work is named for
 * the Apple Account it is for (the engine's account tag, kept on the phone account), so one
 * account's work is never mistaken for, kept by, or cancelled with another's.
 */
object Work {
  /** The engine's account tag a phone account was made for (see [ensureAccount]). */
  fun tagOf(context: Context, account: Account): String? =
    runCatching { AccountManager.get(context).getUserData(account, ACCOUNT_OWNER) }.getOrNull()

  /** A piece of work's unique name: what it is and whose ("sync:<tag>:<authority>"). */
  fun name(kind: String, tag: String, what: String? = null): String =
    if (what == null) "$kind:$tag" else "$kind:$tag:$what"

  /** The tag every piece of one account's work carries, to cancel it all at once. */
  fun accountTag(tag: String) = "account:$tag"

  /**
   * Where work is handed to WorkManager from the screens, off the main thread: handing it over
   * reads WorkManager's own database. What is handed over then outlives the process.
   */
  val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

  private val enqueuing = Mutex()

  /**
   * `request` run under `name`, and once more after one running now, never more than once
   * more: DAVx5's way (SyncWorkerManager.enqueueOneTime). Ten changes made while a sync runs
   * are one sync after it, which reads all ten; a request made while one is already waiting
   * adds nothing.
   */
  suspend fun enqueueOnceMore(context: Context, name: String, request: OneTimeWorkRequest) {
    val manager = WorkManager.getInstance(context)
    enqueuing.withLock {
      val now = manager.getWorkInfosForUniqueWorkFlow(name).first()
      if (now.any { it.state == WorkInfo.State.ENQUEUED || it.state == WorkInfo.State.BLOCKED }) return
      manager.enqueueUniqueWork(name, ExistingWorkPolicy.APPEND_OR_REPLACE, request)
    }
  }

  /** Everything one account has waiting or running in the background, stopped. */
  fun cancelAll(context: Context, tag: String) {
    WorkManager.getInstance(context).cancelAllWorkByTag(accountTag(tag))
  }

  private const val CHANNEL = "background"

  /**
   * What the notification says while work runs in the foreground: only where Android needs
   * one (an expedited job before Android 12 runs as a foreground service; Send to iPhone is
   * one the person started and watches).
   */
  fun foreground(context: Context, id: Int, title: String, channel: String = CHANNEL, progress: Pair<Int, Int>? = null): ForegroundInfo {
    val manager = context.getSystemService(NotificationManager::class.java)
    if (manager.getNotificationChannel(channel) == null) {
      manager.createNotificationChannel(
        when (channel) {
          CHANNEL -> NotificationChannel(CHANNEL, "Syncing", NotificationManager.IMPORTANCE_MIN).apply {
            description = "Shown while Skykeep syncs with iCloud, only when Android asks for it"
          }
          else -> NotificationChannel(channel, "Uploads and downloads", NotificationManager.IMPORTANCE_LOW).apply {
            description = "Files sent to your iPhone, and how it went"
          }
        },
      )
    }
    val note = NotificationCompat.Builder(context, channel)
      .setSmallIcon(R.drawable.ic_upload)
      .setContentTitle(title)
      .setOngoing(true)
      .setSilent(true)
      .apply { if (progress != null) setProgress(progress.second, progress.first, progress.second == 0) }
      .build()
    return ForegroundInfo(id, note, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
  }
}

/**
 * Whether a throwable is the engine failing a call: refused or unreachable (EngineException),
 * or a fault inside it (InternalException: UniFFI turns a panic in Rust into one). Either fails
 * that one piece of work, recorded and tried again later; neither may take the app down.
 */
fun Throwable.fromEngine(): Boolean =
  this is uniffi.quince_mobile.EngineException || this is uniffi.quince_mobile.InternalException

/** What a failure from the engine says, in words for the screen. */
fun Throwable.reason(otherwise: String): String =
  (this as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: otherwise

package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.content.ContentResolver
import android.content.Context
import android.provider.CalendarContract
import android.provider.ContactsContract
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.Data
import androidx.work.ListenableWorker.Result as WorkResult
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import io.github.yashwanthkamireddi.skykeep.Engine
import uniffi.quince_mobile.SessionState
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import kotlin.time.Duration
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/** How one service's sync pass ended. */
enum class Pass {
  /** Done: nothing more until the next one. */
  Done,

  /** More to read: the next pass at once (a large Photos library, a page at a time). */
  More,

  /** Not all of it went (Apple busy or unreachable, changes still to send): again, backing off. */
  Again,
}

/**
 * Syncing as WorkManager runs it, the way DAVx5 does since it left the sync framework's own
 * scheduling (SyncWorkerManager, OneTimeSyncWorker, PeriodicSyncWorker): one piece of work per
 * account and service, at once when asked (a change made, Sync Now) and on the interval chosen
 * in Settings, each with its network rule, exponential backoff, and Android's limits on time and
 * battery. The sync adapters stay: Contacts and Calendar need them for Android's own apps, and
 * the account's sync settings (each service on or off) still decide.
 */
object SyncWork {
  private const val AUTHORITY = "authority"
  private const val ACCOUNT = "account"
  private const val PERIODIC = "periodic"

  /** The tag on every sync, to say which services are syncing now. */
  const val SYNCING = "sync"

  fun authorityTag(authority: String) = "authority:$authority"

  /**
   * One pass per service at a time, whichever piece of work asked: one asked for and the one
   * on the interval may both start, and a second Contacts pass beside the first would write
   * the same rows twice.
   */
  private val lanes = ConcurrentHashMap<String, Mutex>()

  internal fun lane(authority: String): Mutex = lanes.getOrPut(authority) { Mutex() }

  /**
   * A sync of `authority` for `account` as soon as there is a network: expedited where Android
   * allows (a change made here goes at once), else as ordinary work. Answers the work's name,
   * or null for a phone account no Apple Account owns.
   */
  suspend fun now(context: Context, account: Account, authority: String): String? {
    val tag = Work.tagOf(context, account) ?: return null
    val name = Work.name("sync", tag, authority)
    val request = OneTimeWorkRequestBuilder<SyncWorker>()
      .setInputData(workDataOf(AUTHORITY to authority, ACCOUNT to account.name))
      .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
      .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
      .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
      .addTag(SYNCING).addTag(authorityTag(authority)).addTag(Work.accountTag(tag))
      .build()
    Work.enqueueOnceMore(context, name, request)
    return name
  }

  /** Until the newest piece of work under `name` has finished, or `limit` has passed. */
  suspend fun waitFor(context: Context, name: String, limit: Duration) {
    val manager = WorkManager.getInstance(context)
    val last = manager.getWorkInfosForUniqueWorkFlow(name).first().lastOrNull { !it.state.isFinished } ?: return
    withTimeoutOrNull(limit) { manager.getWorkInfoByIdFlow(last.id).first { it == null || it.state.isFinished } }
  }

  /**
   * Every service synced on its own every `seconds`, on any network, never with the battery
   * low; Android picks the moment within it. Replaces the sync framework's periodic syncs,
   * which are taken off the account so a service is never synced twice over.
   */
  fun every(context: Context, account: Account, tag: String, seconds: Long) {
    val manager = WorkManager.getInstance(context)
    for (authority in SYNCED) {
      ContentResolver.removePeriodicSync(account, authority, android.os.Bundle.EMPTY)
      val request = PeriodicWorkRequestBuilder<SyncWorker>(seconds, TimeUnit.SECONDS)
        .setInputData(workDataOf(AUTHORITY to authority, ACCOUNT to account.name, PERIODIC to true))
        .setConstraints(
          Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).setRequiresBatteryNotLow(true).build(),
        )
        .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 30, TimeUnit.SECONDS)
        .addTag(SYNCING).addTag(authorityTag(authority)).addTag(Work.accountTag(tag))
        .build()
      manager.enqueueUniquePeriodicWork(Work.name("sync-every", tag, authority), ExistingPeriodicWorkPolicy.UPDATE, request)
    }
  }

  /** Whether the account's own sync settings let a service sync on its own now. */
  private fun wanted(account: Account, authority: String): Boolean =
    ContentResolver.getMasterSyncAutomatically() &&
      ContentResolver.getSyncAutomatically(account, authority) &&
      ContentResolver.getIsSyncable(account, authority) > 0

  /** One pass of a service, by its authority. */
  internal fun pass(context: Context, engine: uniffi.quince_mobile.AccountInterface, account: Account, authority: String): Pass =
    when (authority) {
      CalendarContract.AUTHORITY -> calendarPass(context, engine, account, authority)
      ContactsContract.AUTHORITY -> contactsPass(context, engine, account, authority)
      REMINDERS_AUTHORITY -> remindersPass(context, engine, authority)
      NOTES_AUTHORITY -> notesPass(context, engine, authority)
      PHOTOS_AUTHORITY -> photosPass(context, engine, authority)
      else -> Pass.Done
    }

  internal suspend fun run(context: Context, input: Data, attempt: Int): WorkResult {
    val authority = input.getString(AUTHORITY) ?: return WorkResult.failure()
    val account = Account(input.getString(ACCOUNT) ?: return WorkResult.failure(), ACCOUNT_TYPE)
    val engine = Engine.account(context)
    // Another Apple Account's phone account, or nobody signed in: nothing of it is read with
    // this session, and its work stops until its own account is signed in again.
    if (refuseOthers(context, engine, account, authority)) {
      Work.tagOf(context, account)?.let { tag -> if (tag != engine.accountTag()) Work.cancelAll(context, tag) }
      return WorkResult.success()
    }
    if (input.getBoolean(PERIODIC, false) && !wanted(account, authority)) return WorkResult.success()
    // The session kept alive before the pass, as the desktop keeps it: taken up again by itself
    // when it lapsed. Ended by Apple, nothing more is tried until the person signs in again;
    // before, every sync after the cookies lapsed failed quietly for good.
    val tag = Work.tagOf(context, account) ?: return WorkResult.success()
    if (SessionEnded.ended(context, tag)) return WorkResult.success()
    when (withContext(Dispatchers.IO) { engine.ensureSession() }) {
      SessionState.ALIVE -> {}
      // Offline, or Apple busy: the same backoff as a pass that couldn't reach Apple.
      SessionState.AWAY -> return if (attempt < 5) WorkResult.retry() else WorkResult.success()
      SessionState.CODE, SessionState.SIGN_IN -> { SessionEnded.mark(context, tag); return WorkResult.success() }
      SessionState.SIGNED_OUT -> return WorkResult.success()
    }
    val pass =lane(authority).withLock { withContext(Dispatchers.IO) { pass(context, engine, account, authority) } }
    return when (pass) {
      Pass.Done -> WorkResult.success()
      Pass.More -> { now(context, account, authority); WorkResult.success() }
      // Backing off from 30 s, a few times; past that the next sync on the interval tries.
      Pass.Again -> if (attempt < 5) WorkResult.retry() else WorkResult.success()
    }
  }
}

/** One sync pass of one service, as [SyncWork] asked for it. */
class SyncWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
  override suspend fun doWork(): Result = SyncWork.run(applicationContext, inputData, runAttemptCount)

  /** Before Android 12 an expedited job runs as a foreground service, which needs a notification. */
  override suspend fun getForegroundInfo() = Work.foreground(applicationContext, 7102, "Syncing with iCloud")
}

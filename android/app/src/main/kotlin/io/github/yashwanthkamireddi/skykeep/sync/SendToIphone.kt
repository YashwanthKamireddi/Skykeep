package io.github.yashwanthkamireddi.skykeep.sync

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.job.JobInfo
import android.app.job.JobParameters
import android.app.job.JobScheduler
import android.app.job.JobService
import android.content.ComponentName
import android.content.Context
import android.os.Build
import android.os.PersistableBundle
import androidx.annotation.RequiresApi
import androidx.core.app.NotificationCompat
import androidx.work.CoroutineWorker
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.OutOfQuotaPolicy
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.ExistingWorkPolicy
import androidx.work.Constraints
import androidx.work.NetworkType
import androidx.work.workDataOf
import io.github.yashwanthkamireddi.skykeep.Engine
import io.github.yashwanthkamireddi.skykeep.R
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import uniffi.quince_mobile.PBackup

/**
 * Send to iPhone, as the desktop's: each file to where the iPhone shows it. A photo or video
 * iCloud Photos takes goes to Photos; anything else (a video too large for Photos included) to
 * iCloud Drive, in a folder named after this phone ("From Pixel 9"), so nothing sent is refused
 * for its kind. Nothing is asked of the iPhone.
 *
 * The person started it and is waiting for it, so it runs as Android has such transfers run: on
 * Android 14 and later a user-initiated data transfer job (JobScheduler, the only kind of
 * background work not cut off after ten minutes and not deferred, shown in the notification shade
 * and the Task Manager), earlier as expedited WorkManager work in the foreground. Either way it
 * outlives the Share sheet and the process: the shared files wait in cache/send/<batch>/, one
 * folder each ("<n>/<name>"), each deleted once it has gone, so work started again after the
 * process was killed sends only what had not gone yet.
 */
object SendToIphone {
  private const val CHANNEL = "transfers"
  private const val NOTE = 7101
  private const val SENDING = 7103

  /** Where a batch's shared copies wait until they have gone. */
  fun batches(context: Context) = File(context.cacheDir, "send")

  /** The iCloud Drive folder sent files land in: "From" and this phone's name. */
  fun folderName(): String = "From ${Build.MODEL.ifBlank { "Android phone" }}"

  /**
   * Sends the files waiting in `batch` (copies made for this share), for the Apple Account
   * signed in now (`tag`). Called while the Share sheet is on screen: Android lets a
   * user-initiated job be scheduled only by an app the person can see.
   */
  fun start(context: Context, batch: File, tag: String) {
    val app = context.applicationContext
    if (Build.VERSION.SDK_INT >= 34 && scheduleJob(app, batch, tag)) return
    val request = OneTimeWorkRequestBuilder<SendWorker>()
      .setInputData(workDataOf("batch" to batch.name, "tag" to tag))
      .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
      .setExpedited(OutOfQuotaPolicy.RUN_AS_NON_EXPEDITED_WORK_REQUEST)
      .addTag(Work.accountTag(tag))
      .build()
    WorkManager.getInstance(app).enqueueUniqueWork("send:${batch.name}", ExistingWorkPolicy.KEEP, request)
  }

  /** A batch's job id: in a range of its own, far above the ids WorkManager counts up from 0. */
  internal fun jobId(batch: String) = 0x5E00_0000 + (batch.hashCode() and 0xF_FFFF)

  @RequiresApi(34)
  private fun scheduleJob(context: Context, batch: File, tag: String): Boolean {
    val bytes = items(batch).sumOf { it.length() }
    fun info(persisted: Boolean) = JobInfo.Builder(jobId(batch.name), ComponentName(context, SendJob::class.java))
      .setUserInitiated(true)
      .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
      .setEstimatedNetworkBytes(JobInfo.NETWORK_BYTES_UNKNOWN.toLong(), bytes)
      .setPersisted(persisted)
      .setExtras(PersistableBundle().apply { putString("batch", batch.name); putString("tag", tag) })
      .build()
    val scheduler = context.getSystemService(JobScheduler::class.java)
    // Kept across a restart where Android allows it; a refusal (not visible, the permission
    // withdrawn) falls back to WorkManager.
    return runCatching { scheduler.schedule(info(true)) == JobScheduler.RESULT_SUCCESS }
      .recoverCatching { scheduler.schedule(info(false)) == JobScheduler.RESULT_SUCCESS }
      .getOrDefault(false)
  }

  /** The files still waiting in a batch, in the order shared. */
  internal fun items(batch: File): List<File> =
    (batch.listFiles { f -> f.isDirectory } ?: emptyArray())
      .sortedBy { it.name.toIntOrNull() ?: Int.MAX_VALUE }
      .mapNotNull { dir -> dir.listFiles()?.firstOrNull { it.isFile } }

  /**
   * What has gone so far, kept beside the files so the notification at the end counts what went
   * before a restart too: one line per file, "P" (Photos), "D" (Drive) or "F<tab>reason".
   */
  private fun outcome(batch: File) = File(batch, ".outcome")

  /**
   * Every file still waiting in `batch` sent, each deleted once it has gone, then one
   * notification says how it ended. `tag` is the Apple Account it was shared to: signed out or
   * another account signed in since, nothing goes to it. `progress` is told after each file.
   */
  fun send(context: Context, batch: File, tag: String?, progress: (Int, Int) -> Unit = { _, _ -> }) {
    val engine = Engine.account(context)
    if (tag == null || engine.accountTag() != tag) {
      tell(context, "Nothing was sent", detail = "Sign in to Skykeep again, then share again.")
      batch.deleteRecursively()
      return
    }
    val todo = items(batch)
    val log = outcome(batch)
    var folder: String? = null
    todo.forEachIndexed { i, file ->
      progress(i, todo.size)
      val name = file.name
      // A key of its own per batch and place: sent again after a restart, Photos takes it as
      // the same photo (Duplicate), never a second copy.
      val key = "send:${batch.name}/${file.parentFile?.name}"
      val line = try {
        if (engine.photoAccepts(name, file.length().toULong())) {
          when (val r = engine.photoBackup(key, file.path, name)) {
            is PBackup.Sent, is PBackup.Duplicate -> "P"
            is PBackup.Refused -> "F\t$name: ${r.reason}"
          }
        } else {
          val into = folder ?: run {
            val root = engine.driveRoot()
            val want = folderName()
            (engine.driveChildren(root, true).firstOrNull { it.folder && it.name.equals(want, ignoreCase = true) }?.id
              ?: engine.driveMkdir(root, want).id).also { folder = it }
          }
          engine.driveWriteBack(into, file.path)
          "D"
        }
      } catch (e: Exception) {
        if (!e.fromEngine()) throw e
        "F\t$name: ${e.reason("it couldn't be sent")}"
      }
      log.appendText(line + "\n")
      file.parentFile?.deleteRecursively()
    }
    val lines = runCatching { log.readLines() }.getOrDefault(emptyList()).filter { it.isNotEmpty() }
    batch.deleteRecursively()
    val photos = lines.count { it == "P" }
    val drive = lines.count { it == "D" }
    val failed = lines.filter { it.startsWith("F\t") }.map { it.removePrefix("F\t") }
    val where = listOfNotNull(
      if (photos > 0) "${count(photos)} in Photos" else null,
      if (drive > 0) "${count(drive)} in iCloud Drive, in “${folderName()}”" else null,
    ).joinToString(", ")
    when {
      lines.isEmpty() -> Unit
      failed.isEmpty() -> tell(context, "Sent to your iPhone", detail = where)
      photos + drive == 0 -> tell(context, "Nothing could be sent", detail = failed.first())
      else -> tell(context, "${failed.size} of ${lines.size} couldn't be sent", detail = "$where. ${failed.first()}")
    }
  }

  private fun count(n: Int) = if (n == 1) "1 item" else "$n items"

  private fun channel(context: Context): NotificationManager {
    val manager = context.getSystemService(NotificationManager::class.java)
    if (manager.getNotificationChannel(CHANNEL) == null) {
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL, "Uploads and downloads", NotificationManager.IMPORTANCE_LOW).apply {
          description = "Files sent to your iPhone, and how it went"
        },
      )
    }
    return manager
  }

  /** The notification while it sends: how many have gone of how many. */
  internal fun sending(context: Context, done: Int, total: Int): android.app.Notification {
    channel(context)
    return NotificationCompat.Builder(context, CHANNEL)
      .setSmallIcon(R.drawable.ic_upload)
      .setContentTitle(if (total <= 1) "Sending to your iPhone" else "Sending ${done + 1} of $total to your iPhone")
      .setOngoing(true)
      .setSilent(true)
      .setProgress(total, done, total <= 1)
      .build()
  }

  internal const val SENDING_ID = SENDING

  private fun tell(context: Context, title: String, detail: String? = null) {
    val manager = channel(context)
    val note = NotificationCompat.Builder(context, CHANNEL)
      .setSmallIcon(R.drawable.ic_upload)
      .setContentTitle(title)
      .apply { detail?.let { setContentText(it).setStyle(NotificationCompat.BigTextStyle().bigText(it)) } }
      .setAutoCancel(true)
      .build()
    runCatching { manager.notify(NOTE, note) }
  }
}

/**
 * Send to iPhone on Android 14 and later: a user-initiated data transfer job. Its notification
 * is shown from the start (Android requires it) and taken away at the end, when the one saying
 * how it went appears. Stopped by Android (the network gone), it is run again later and carries
 * on; stopped by the person (Task Manager), the files waiting are let go of.
 */
@RequiresApi(34)
class SendJob : JobService() {
  private val running = ConcurrentHashMap<Int, Job>()

  override fun onStartJob(params: JobParameters): Boolean {
    val name = params.extras.getString("batch") ?: return false
    val batch = File(SendToIphone.batches(this), name)
    setNotification(params, SendToIphone.SENDING_ID, SendToIphone.sending(this, 0, 0), JOB_END_NOTIFICATION_POLICY_REMOVE)
    running[params.jobId] = Work.scope.launch {
      try {
        SendToIphone.send(applicationContext, batch, params.extras.getString("tag")) { done, total ->
          runCatching { setNotification(params, SendToIphone.SENDING_ID, SendToIphone.sending(this@SendJob, done, total), JOB_END_NOTIFICATION_POLICY_REMOVE) }
        }
      } finally {
        running.remove(params.jobId)
        jobFinished(params, false)
      }
    }
    return true
  }

  override fun onStopJob(params: JobParameters): Boolean {
    running.remove(params.jobId)?.cancel()
    val byPerson = params.stopReason == JobParameters.STOP_REASON_USER
    if (byPerson) params.extras.getString("batch")?.let { File(SendToIphone.batches(this), it).deleteRecursively() }
    return !byPerson
  }
}

/** Send to iPhone before Android 14: expedited work, in the foreground while it sends. */
class SendWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
  override suspend fun doWork(): Result {
    val name = inputData.getString("batch") ?: return Result.success()
    val batch = File(SendToIphone.batches(applicationContext), name)
    runCatching { setForeground(getForegroundInfo()) }
    SendToIphone.send(applicationContext, batch, inputData.getString("tag")) { done, total ->
      runCatching { setForegroundAsync(info(done, total)) }
    }
    return Result.success()
  }

  private fun info(done: Int, total: Int) =
    androidx.work.ForegroundInfo(
      SendToIphone.SENDING_ID, SendToIphone.sending(applicationContext, done, total),
      android.content.pm.ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
    )

  override suspend fun getForegroundInfo() = info(0, 0)
}

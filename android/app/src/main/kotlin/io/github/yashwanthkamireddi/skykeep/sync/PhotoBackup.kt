package io.github.yashwanthkamireddi.skykeep.sync

import android.content.ContentUris
import android.content.Context
import android.os.SystemClock
import android.provider.MediaStore
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import androidx.work.workDataOf
import java.io.File
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.PBackup

/**
 * New photos and films taken on this phone backed up to iCloud Photos, when the person turns it
 * on (off by default). From the moment it is turned on: what was on the phone before stays as it
 * is unless sent by hand, so turning it on never floods iCloud with a whole camera roll at once.
 * Each is sent in the order taken, by the desktop's own upload (the engine's `photo_backup`),
 * never twice; the place reached is kept, so each run carries on where the last one stopped.
 *
 * It runs as its own WorkManager work, as Immich and Nextcloud back up: started when MediaStore
 * says a photo or film was added (a content-URI trigger, armed again after each), with a run
 * every few hours in case one was missed, on Wi-Fi unless mobile data is allowed, never with the
 * battery low, and only while charging when the person asks. A run stops after [BUDGET_MS], well
 * inside Android's ten minutes for a job, and the next carries on at once.
 */
object PhotoBackup {
  private const val PREFS = "photo-backup"

  /** How long one run sends before it hands over to the next. */
  const val BUDGET_MS = 9L * 60 * 1000

  /** How often a run comes on its own, whatever MediaStore said. */
  private const val EVERY_HOURS = 6L

  private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

  fun on(context: Context) = prefs(context).getBoolean("on", false)

  /** Whether backing up may use mobile data. Off by default: Wi-Fi (or any unmetered network) only. */
  fun mobileData(context: Context) = prefs(context).getBoolean("mobileData", false)

  fun setMobileData(context: Context, allowed: Boolean) {
    prefs(context).edit().putBoolean("mobileData", allowed).apply()
    schedule(context)
  }

  /** Whether backing up waits for the phone to be charging. Off by default. */
  fun whileCharging(context: Context) = prefs(context).getBoolean("whileCharging", false)

  fun setWhileCharging(context: Context, only: Boolean) {
    prefs(context).edit().putBoolean("whileCharging", only).apply()
    schedule(context)
  }

  /** On a network that costs nothing per byte, or one the person allowed. */
  private fun mayUpload(context: Context): Boolean {
    if (mobileData(context)) return true
    val cm = context.getSystemService(android.net.ConnectivityManager::class.java) ?: return false
    return !cm.isActiveNetworkMetered
  }

  /** Turned on (from now) or off. */
  fun set(context: Context, on: Boolean) {
    prefs(context).edit().putBoolean("on", on).apply {
      if (on && !prefs(context).contains("since")) putLong("since", System.currentTimeMillis() / 1000)
    }.commit()
    schedule(context)
    if (on) Work.scope.launch { now(context) }
  }

  /** The signed-in account's tag, from its phone account: what backup's work is named for. */
  private fun tag(context: Context): String? = currentAccount(context)?.let { Work.tagOf(context, it) }

  /** What a run needs from the phone, as the person chose it. */
  private fun constraints(context: Context) = Constraints.Builder()
    .setRequiredNetworkType(if (mobileData(context)) NetworkType.CONNECTED else NetworkType.UNMETERED)
    .setRequiresBatteryNotLow(true)
    .setRequiresCharging(whileCharging(context))
    .build()

  /**
   * Backup's work made to match its setting: when on, the watch on MediaStore armed (kept if it
   * is) and the run every few hours set to the chosen conditions; when off, all of it stopped.
   */
  fun schedule(context: Context) {
    val tag = tag(context) ?: return
    val manager = WorkManager.getInstance(context)
    if (!on(context)) {
      for (kind in listOf("photo-watch", "photo-backup", "photo-backup-every")) manager.cancelUniqueWork(Work.name(kind, tag))
      return
    }
    watch(context, tag, again = false)
    val every = PeriodicWorkRequestBuilder<PhotoBackupWorker>(EVERY_HOURS, TimeUnit.HOURS)
      .setInputData(workDataOf("tag" to tag))
      .setConstraints(constraints(context))
      .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
      .addTag(Work.accountTag(tag))
      .build()
    manager.enqueueUniquePeriodicWork(Work.name("photo-backup-every", tag), ExistingPeriodicWorkPolicy.UPDATE, every)
  }

  /**
   * The watch on MediaStore: one piece of work that runs when a photo or film is added (a few
   * seconds after the last of a burst), asks for a run, and arms itself again. `again` is the
   * watch arming the next while it runs (appended after itself); from anywhere else an armed
   * watch is kept as it is, so app starts never pile them up.
   */
  internal fun watch(context: Context, tag: String, again: Boolean) {
    val request = OneTimeWorkRequestBuilder<PhotoWatchWorker>()
      .setInputData(workDataOf("tag" to tag))
      .setConstraints(
        Constraints.Builder()
          .addContentUriTrigger(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, true)
          .addContentUriTrigger(MediaStore.Video.Media.EXTERNAL_CONTENT_URI, true)
          .setTriggerContentUpdateDelay(5, TimeUnit.SECONDS)
          .setTriggerContentMaxDelay(1, TimeUnit.MINUTES)
          .build(),
      )
      .addTag(Work.accountTag(tag))
      .build()
    WorkManager.getInstance(context).enqueueUniqueWork(
      Work.name("photo-watch", tag), if (again) ExistingWorkPolicy.APPEND_OR_REPLACE else ExistingWorkPolicy.KEEP, request,
    )
  }

  /** A run as soon as the chosen conditions allow, and once more after one running now. */
  suspend fun now(context: Context) {
    if (!on(context)) return
    val tag = tag(context) ?: return
    val request = OneTimeWorkRequestBuilder<PhotoBackupWorker>()
      .setInputData(workDataOf("tag" to tag))
      .setConstraints(constraints(context))
      .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 1, TimeUnit.MINUTES)
      .addTag(Work.accountTag(tag))
      .build()
    Work.enqueueOnceMore(context, Work.name("photo-backup", tag), request)
  }

  /** One run at a time, whichever work started it: they share the place reached. */
  internal val lane = Mutex()

  private data class Taken(val key: String, val uri: android.net.Uri, val name: String, val added: Long, val generation: Long)

  /**
   * What may need backing up, oldest change first: photos and films taken since backup was
   * turned on. On Android 11 and later by MediaStore's generation (a counter every insert and
   * change moves), which neither a burst in one second nor a film still being written can slip
   * past: a film finishes with a newer generation and is seen then. A MediaStore reset (its
   * version changes) starts the count again; the keys carry the version, so ids reused after a
   * reset are never taken for another photo ("phone|version|content uri"). On Android 10, by the second it was added, the
   * last second asked again (the engine never sends one key twice).
   */
  private fun changed(context: Context, p: android.content.SharedPreferences, since: Long): List<Taken> {
    val modern = android.os.Build.VERSION.SDK_INT >= 30
    val version = MediaStore.getVersion(context)
    if (p.getString("version", null) != version) p.edit().putString("version", version).putLong("generation", 0).apply()
    val out = ArrayList<Taken>()
    for (collection in listOf(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, MediaStore.Video.Media.EXTERNAL_CONTENT_URI)) {
      val columns = arrayListOf(MediaStore.MediaColumns._ID, MediaStore.MediaColumns.DISPLAY_NAME, MediaStore.MediaColumns.DATE_ADDED)
      val (where, args) = if (modern) {
        columns += MediaStore.MediaColumns.GENERATION_MODIFIED
        "${MediaStore.MediaColumns.GENERATION_MODIFIED} > ? AND ${MediaStore.MediaColumns.DATE_ADDED} >= ?" to
          arrayOf(p.getLong("generation", 0).toString(), since.toString())
      } else {
        "${MediaStore.MediaColumns.DATE_ADDED} >= ?" to arrayOf(maxOf(since, p.getLong("added", since) - 1).toString())
      }
      context.contentResolver.query(collection, columns.toTypedArray(), where, args, null)?.use { c ->
        while (c.moveToNext()) {
          val id = c.getLong(0)
          out += Taken(
            "phone|$version|${ContentUris.withAppendedId(collection, id)}", ContentUris.withAppendedId(collection, id),
            c.getString(1) ?: "photo", c.getLong(2), if (modern) c.getLong(3) else 0,
          )
        }
      }
    }
    return if (modern) out.sortedBy { it.generation } else out.sortedBy { it.added }
  }

  /**
   * The photo's bytes as taken: with the location kept when the person allowed it (Android 10+
   * removes a photo's GPS from what an app reads unless it holds ACCESS_MEDIA_LOCATION and asks
   * for the original), so a backed-up photo still knows where it was taken.
   */
  private fun original(context: Context, uri: android.net.Uri): android.net.Uri =
    if (androidx.core.content.ContextCompat.checkSelfPermission(context, android.Manifest.permission.ACCESS_MEDIA_LOCATION) ==
      android.content.pm.PackageManager.PERMISSION_GRANTED
    ) MediaStore.setRequireOriginal(uri) else uri

  /** How a run ended. */
  enum class Ended {
    /** Everything taken so far is in iCloud (or set aside to try again). */
    Done,

    /** Out of time, or Android asked the work to stop: the next run carries on. */
    More,

    /** Left Wi-Fi (mobile data not allowed): carried on once back on it. */
    Offline,
  }

  /**
   * One run's backing up, until `until` (SystemClock.elapsedRealtime) or `stopped`. Each photo
   * on its own: one that fails is kept aside and tried again next time (up to [Tries.MOST]
   * times), never holding back the ones after it. The network is checked before each file, so a
   * phone that leaves Wi-Fi mid-way stops before the next one.
   */
  fun run(context: Context, until: Long, stopped: () -> Boolean): Ended {
    if (!on(context)) return Ended.Done
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
    val p = prefs(context)
    val since = p.getLong("since", Long.MAX_VALUE)
    val retry = p.getStringSet("retry", emptySet())!!.toMutableSet()
    val tries = Tries.decode(p.getString("tries", "")!!)
    fun out(): Boolean = SystemClock.elapsedRealtime() >= until || stopped()
    fun keep() = p.edit().putStringSet("retry", retry.toSet()).putString("tries", Tries.encode(tries)).apply()

    for (t in changed(context, p, since)) {
      if (out()) return Ended.More
      if (!mayUpload(context)) return Ended.Offline
      if (engine.photoBackupDone(t.key)) { advance(p, t); continue }
      val outcome = send(context, engine, t.key, t.uri, t.name)
      if (outcome == null) {
        // Not now (offline, Apple busy, a file that can't be read): tried again next run, a
        // few times at most, and never in the way of the photos after it.
        val n = Tries.failed(tries, t.key)
        if (n < Tries.MOST) retry += t.key else retry -= t.key
        android.util.Log.w("Skykeep", "A photo wasn't backed up this time (try $n of ${Tries.MOST})")
      } else {
        if (outcome is PBackup.Refused) android.util.Log.w("Skykeep", "A photo wasn't backed up: ${outcome.reason}")
        tries.remove(t.key); retry -= t.key
      }
      advance(p, t)
      keep()
    }
    // Those kept aside are asked again: the generation has moved past them.
    for (key in retry.toList()) {
      if (out()) { keep(); return Ended.More }
      if (!mayUpload(context)) { keep(); return Ended.Offline }
      val uri = android.net.Uri.parse(key.substringAfterLast('|'))
      val name = context.contentResolver.query(uri, arrayOf(MediaStore.MediaColumns.DISPLAY_NAME), null, null, null)
        ?.use { c -> if (c.moveToFirst()) c.getString(0) else null }
      if (name == null) { retry -= key; tries.remove(key); continue } // gone from the phone
      val outcome = send(context, engine, key, uri, name)
      val n = Tries.failed(tries, key)
      if (outcome != null || n >= Tries.MOST) { retry -= key; tries.remove(key) }
    }
    keep()
    return Ended.Done
  }

  /** One photo copied out of MediaStore and sent; null when it couldn't be, this time. */
  private fun send(context: Context, engine: uniffi.quince_mobile.AccountInterface, key: String, uri: android.net.Uri, name: String): PBackup? {
    val dir = File(context.cacheDir, "backup/${DriveFiles.keyOf(key)}").also { it.mkdirs() }
    val file = File(dir, DriveFiles.safeName(name))
    return try {
      runCatching {
        context.contentResolver.openInputStream(original(context, uri))?.use { input -> file.outputStream().use { input.copyTo(it) } }
          ?: return null
        engine.photoBackup(key, file.path, name)
      }.getOrNull()
    } finally {
      dir.deleteRecursively()
    }
  }

  /** The place reached: past this photo, by generation (Android 11+) or by the second it was added. */
  private fun advance(p: android.content.SharedPreferences, t: Taken) {
    p.edit().apply {
      if (t.generation > p.getLong("generation", 0)) putLong("generation", t.generation)
      if (t.added > p.getLong("added", 0)) putLong("added", t.added)
    }.apply()
  }
}

/**
 * How many times each photo kept aside has failed, kept in one preference as "key=n,key=n". A
 * key is "phone|version|content://…", which holds no comma; its last "=" is the count's.
 */
object Tries {
  /** Tries before a photo is let go of (it stays on the phone, and can be sent by hand). */
  const val MOST = 5

  fun decode(kept: String): MutableMap<String, Int> =
    kept.split(',').filter { it.contains('=') }
      .mapNotNull { e -> e.substringAfterLast('=').toIntOrNull()?.let { e.substringBeforeLast('=') to it } }
      .toMap().toMutableMap()

  fun encode(tries: Map<String, Int>): String = tries.entries.joinToString(",") { "${it.key}=${it.value}" }

  /** One more failure counted for `key`; answers how many there have been. */
  fun failed(tries: MutableMap<String, Int>, key: String): Int {
    val n = (tries[key] ?: 0) + 1
    if (n < MOST) tries[key] = n else tries.remove(key)
    return n
  }
}

/** One run of photo backup, for the account it was asked for only. */
class PhotoBackupWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
  override suspend fun doWork(): Result {
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(applicationContext)
    // Signed out, or another Apple Account signed in since: this phone's photos go to nobody's
    // iCloud but the account that turned backup on (backup's settings go with its account).
    if (inputData.getString("tag") != engine.accountTag()) return Result.success()
    val until = SystemClock.elapsedRealtime() + PhotoBackup.BUDGET_MS
    val ended = PhotoBackup.lane.withLock {
      withContext(Dispatchers.IO) { PhotoBackup.run(applicationContext, until) { isStopped } }
    }
    return when (ended) {
      PhotoBackup.Ended.Done -> Result.success()
      PhotoBackup.Ended.More -> { PhotoBackup.now(applicationContext); Result.success() }
      PhotoBackup.Ended.Offline -> Result.retry()
    }
  }
}

/** MediaStore said a photo or film was added: a backup run asked for, and the watch armed again. */
class PhotoWatchWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
  override suspend fun doWork(): Result {
    val tag = inputData.getString("tag") ?: return Result.success()
    if (!PhotoBackup.on(applicationContext)) return Result.success()
    PhotoBackup.now(applicationContext)
    PhotoBackup.watch(applicationContext, tag, again = true)
    return Result.success()
  }
}

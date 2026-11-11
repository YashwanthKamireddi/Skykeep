package io.github.yashwanthkamireddi.skykeep.sync

import android.content.ContentUris
import android.content.Context
import android.provider.MediaStore
import java.io.File
import uniffi.quince_mobile.PBackup

/**
 * New photos and films taken on this phone backed up to iCloud Photos, when the person turns it
 * on (off by default). From the moment it is turned on: what was on the phone before stays as it
 * is unless sent by hand, so turning it on never floods iCloud with a whole camera roll at once.
 * Each is sent in the order taken, by the desktop's own upload (the engine's `photo_backup`),
 * never twice; the place reached is kept, so a sync carries on where the last one stopped.
 */
object PhotoBackup {
  private const val PREFS = "photo-backup"

  private fun prefs(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

  fun on(context: Context) = prefs(context).getBoolean("on", false)

  /** Whether backing up may use mobile data. Off by default: Wi-Fi (or any unmetered network) only. */
  fun mobileData(context: Context) = prefs(context).getBoolean("mobileData", false)

  fun setMobileData(context: Context, allowed: Boolean) = prefs(context).edit().putBoolean("mobileData", allowed).apply()

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
    }.apply()
  }

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

  /**
   * One sync's backing up. Each photo on its own: one that fails is kept aside and tried again
   * next time (up to five times), never holding back the ones after it. The network is checked
   * before each file, so a phone that leaves Wi-Fi mid-way stops before the next one.
   */
  fun run(context: Context) {
    if (!on(context)) return
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
    val p = prefs(context)
    val since = p.getLong("since", Long.MAX_VALUE)
    val retry = p.getStringSet("retry", emptySet())!!.toMutableSet()
    val tries = p.getString("tries", "")!!.split(',').filter { it.contains('=') }
      .associate { it.substringBeforeLast('=') to it.substringAfterLast('=').toInt() }.toMutableMap()
    val todo = changed(context, p, since)
    for (t in todo) {
      if (!mayUpload(context)) return
      if (engine.photoBackupDone(t.key)) { advance(p, t); continue }
      val dir = File(context.cacheDir, "backup/${DriveFiles.keyOf(t.key)}").also { it.mkdirs() }
      val file = File(dir, DriveFiles.safeName(t.name))
      val ok = runCatching {
        context.contentResolver.openInputStream(original(context, t.uri))?.use { input -> file.outputStream().use { input.copyTo(it) } }
        engine.photoBackup(t.key, file.path, t.name)
      }
      dir.deleteRecursively()
      val outcome = ok.getOrNull()
      if (outcome == null) {
        // Not now (offline, Apple busy, a file that can't be read): tried again next sync, five
        // times at most, and never in the way of the photos after it.
        val n = (tries[t.key] ?: 0) + 1
        if (n < 5) { tries[t.key] = n; retry += t.key } else { tries.remove(t.key); retry -= t.key }
        android.util.Log.w("Skykeep", "A photo wasn't backed up this time (try $n of 5)")
      } else {
        if (outcome is PBackup.Refused) android.util.Log.w("Skykeep", "A photo wasn't backed up: ${outcome.reason}")
        tries.remove(t.key); retry -= t.key
      }
      advance(p, t)
      p.edit().putStringSet("retry", retry).putString("tries", tries.entries.joinToString(",") { "${it.key}=${it.value}" }).apply()
    }
    // Those kept aside are asked again: the generation has moved past them.
    if (retry.isNotEmpty()) {
      for (key in retry.toList()) {
        if (!mayUpload(context)) return
        val uri = android.net.Uri.parse(key.substringAfterLast('|'))
        val name = context.contentResolver.query(uri, arrayOf(MediaStore.MediaColumns.DISPLAY_NAME), null, null, null)
          ?.use { c -> if (c.moveToFirst()) c.getString(0) else null }
        if (name == null) { retry -= key; tries.remove(key); continue } // gone from the phone
        val dir = File(context.cacheDir, "backup/${DriveFiles.keyOf(key)}").also { it.mkdirs() }
        val file = File(dir, DriveFiles.safeName(name))
        val outcome = runCatching {
          context.contentResolver.openInputStream(original(context, uri))?.use { input -> file.outputStream().use { input.copyTo(it) } }
          engine.photoBackup(key, file.path, name)
        }.getOrNull()
        dir.deleteRecursively()
        val n = (tries[key] ?: 0) + 1
        if (outcome != null || n >= 5) { retry -= key; tries.remove(key) } else tries[key] = n
      }
      p.edit().putStringSet("retry", retry).putString("tries", tries.entries.joinToString(",") { "${it.key}=${it.value}" }).apply()
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

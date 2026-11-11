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

  private data class Taken(val key: String, val uri: android.net.Uri, val name: String, val added: Long)

  /** What is new since the place reached, oldest first: photos and films together. */
  private fun newer(context: Context, since: Long): List<Taken> {
    val out = ArrayList<Taken>()
    for (collection in listOf(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, MediaStore.Video.Media.EXTERNAL_CONTENT_URI)) {
      context.contentResolver.query(
        collection, arrayOf(MediaStore.MediaColumns._ID, MediaStore.MediaColumns.DISPLAY_NAME, MediaStore.MediaColumns.DATE_ADDED),
        "${MediaStore.MediaColumns.DATE_ADDED} > ?", arrayOf(since.toString()), "${MediaStore.MediaColumns.DATE_ADDED} ASC",
      )?.use { c ->
        while (c.moveToNext()) {
          val id = c.getLong(0)
          out += Taken("phone:$collection/$id", ContentUris.withAppendedId(collection, id), c.getString(1) ?: "photo", c.getLong(2))
        }
      }
    }
    return out.sortedBy { it.added }
  }

  /** One sync's backing up: until it is done, or one can't be sent now (tried again next sync). */
  fun run(context: Context) {
    // Photos are large: never over mobile data unless the person said so; the next sync on
    // Wi-Fi carries on from where this one stopped.
    if (!on(context) || !mayUpload(context)) return
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
    val p = prefs(context)
    for (t in newer(context, p.getLong("since", Long.MAX_VALUE))) {
      // The engine takes a file: the bytes copied to the app's cache, under the photo's name.
      val dir = File(context.cacheDir, "backup/${t.key.hashCode().toUInt().toString(16)}").also { it.mkdirs() }
      val file = File(dir, DriveFiles.safeName(t.name))
      val ok = runCatching {
        context.contentResolver.openInputStream(t.uri)?.use { input -> file.outputStream().use { input.copyTo(it) } }
        engine.photoBackup(t.key, file.path, t.name)
      }
      dir.deleteRecursively()
      val outcome = ok.getOrNull() ?: return // couldn't send now: carry on from here next sync
      if (outcome is PBackup.Refused) android.util.Log.w("Skykeep", "A photo wasn't backed up: ${outcome.reason}")
      p.edit().putLong("since", t.added).apply()
    }
  }
}

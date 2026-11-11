package io.github.yashwanthkamireddi.skykeep.sync

import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import androidx.core.app.NotificationCompat
import io.github.yashwanthkamireddi.skykeep.Engine
import io.github.yashwanthkamireddi.skykeep.R
import java.io.File
import java.util.UUID
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import uniffi.quince_mobile.EngineException
import uniffi.quince_mobile.PBackup

/**
 * Send to iPhone, as the desktop's: each file to where the iPhone shows it. A photo or video
 * iCloud Photos takes goes to Photos; anything else (a video too large for Photos included) to
 * iCloud Drive, in a folder named after this phone ("From Pixel 9"), so nothing sent is refused
 * for its kind. The files are the share's own copies; the sending carries on when the sheet is
 * closed, and one notification says how it ended. Nothing is asked of the iPhone.
 */
object SendToIphone {
  private const val CHANNEL = "transfers"
  private const val NOTE = 7101
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

  /** The iCloud Drive folder sent files land in: "From" and this phone's name. */
  fun folderName(): String = "From ${Build.MODEL.ifBlank { "Android phone" }}"

  /** Sends `files` (copies made for this share, deleted once sent) and says how it went. */
  fun start(context: Context, files: List<File>) {
    val app = context.applicationContext
    scope.launch {
      val engine = Engine.account(app)
      tell(app, "Sending ${count(files.size)} to your iPhone", ongoing = true)
      var photos = 0
      var drive = 0
      val failed = mutableListOf<String>()
      var folder: String? = null
      for (file in files) {
        val name = file.name
        try {
          if (engine.photoAccepts(name, file.length().toULong())) {
            when (val r = engine.photoBackup("send:${UUID.randomUUID()}", file.path, name)) {
              is PBackup.Sent, is PBackup.Duplicate -> photos++
              is PBackup.Refused -> failed += "$name: ${r.reason}"
            }
          } else {
            val into = folder ?: run {
              val root = engine.driveRoot()
              val want = folderName()
              (engine.driveChildren(root, true).firstOrNull { it.folder && it.name.equals(want, ignoreCase = true) }?.id
                ?: engine.driveMkdir(root, want).id).also { folder = it }
            }
            engine.driveWriteBack(into, file.path, null)
            drive++
          }
        } catch (e: EngineException) {
          failed += "$name: ${(e as? EngineException.Failed)?.reason ?: "it couldn't be sent"}"
        } finally {
          file.parentFile?.deleteRecursively()
        }
      }
      val where = listOfNotNull(
        if (photos > 0) "${count(photos)} in Photos" else null,
        if (drive > 0) "${count(drive)} in iCloud Drive, in “${folderName()}”" else null,
      ).joinToString(", ")
      when {
        failed.isEmpty() -> tell(app, "Sent to your iPhone", detail = where)
        photos + drive == 0 -> tell(app, "Nothing could be sent", detail = failed.first())
        else -> tell(app, "${failed.size} of ${files.size} couldn't be sent", detail = "$where. ${failed.first()}")
      }
    }
  }

  private fun count(n: Int) = if (n == 1) "1 item" else "$n items"

  private fun tell(context: Context, title: String, detail: String? = null, ongoing: Boolean = false) {
    val manager = context.getSystemService(NotificationManager::class.java)
    if (manager.getNotificationChannel(CHANNEL) == null) {
      manager.createNotificationChannel(
        NotificationChannel(CHANNEL, "Uploads and downloads", NotificationManager.IMPORTANCE_LOW).apply {
          description = "Files sent to your iPhone, and how it went"
        },
      )
    }
    val note = NotificationCompat.Builder(context, CHANNEL)
      .setSmallIcon(R.drawable.ic_upload)
      .setContentTitle(title)
      .apply { detail?.let { setContentText(it).setStyle(NotificationCompat.BigTextStyle().bigText(it)) } }
      .setOngoing(ongoing)
      .setProgress(0, 0, ongoing)
      .setAutoCancel(!ongoing)
      .build()
    runCatching { manager.notify(NOTE, note) }
  }
}

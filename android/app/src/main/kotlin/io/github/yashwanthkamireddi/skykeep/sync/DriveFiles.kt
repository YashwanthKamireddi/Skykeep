package io.github.yashwanthkamireddi.skykeep.sync

import android.content.Context
import android.provider.DocumentsContract.Document
import android.webkit.MimeTypeMap
import java.io.File
import uniffi.quince_mobile.DItem

/**
 * Where an iCloud Drive file's bytes live on the phone, and what kind of file it is: one answer
 * for the Files app (DriveProvider) and Skykeep's own Drive screen, so a file opened from either
 * is the same copy, fetched once.
 */
object DriveFiles {
  fun copyOf(context: Context, item: DItem): File =
    File(context.cacheDir, "drive/${item.id.hashCode().toUInt().toString(16)}/${item.name}").also { it.parentFile?.mkdirs() }

  fun mimeOf(item: DItem): String =
    if (item.folder) Document.MIME_TYPE_DIR
    else item.name.substringAfterLast('.', "").lowercase().let { MimeTypeMap.getSingleton().getMimeTypeFromExtension(it) } ?: "application/octet-stream"

  /** Whether the phone's copy is missing or older than Apple's. */
  fun stale(file: File, item: DItem): Boolean =
    !file.exists() || (item.modifiedAt ?: 0) * 1000 > file.lastModified() || file.length() != (item.size ?: file.length())
}

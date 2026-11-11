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
    File(context.cacheDir, "drive/${keyOf(item.id)}/${safeName(item.name)}").also { it.parentFile?.mkdirs() }

  /**
   * A folder or file name for an id (a photo's, a Drive item's, a backup's): SHA-256, so two ids
   * never share one cached picture or copy. A 32-bit hash could, and then a Hidden photo's
   * picture could show for another photo in the grid or go out with a share.
   */
  fun keyOf(id: String): String =
    java.security.MessageDigest.getInstance("SHA-256").digest(id.toByteArray()).take(16).joinToString("") { "%02x".format(it) }

  /**
   * A name from outside (Apple's, or another app's) made safe to be one file's name in a folder
   * of ours: no slash, no NUL, never "." or "..", never empty, at most 200 characters. Without
   * it a name like ".." or "a/../../x" could reach outside the folder it was meant for.
   */
  fun safeName(name: String): String {
    val clean = name.replace('/', '_').replace('\u0000', '_').trim().take(200)
    return if (clean.isEmpty() || clean == "." || clean == "..") "file" else clean
  }

  fun mimeOf(item: DItem): String =
    if (item.folder) Document.MIME_TYPE_DIR
    else item.name.substringAfterLast('.', "").lowercase().let { MimeTypeMap.getSingleton().getMimeTypeFromExtension(it) } ?: "application/octet-stream"

  /** Whether the phone's copy is missing or older than Apple's. */
  fun stale(file: File, item: DItem): Boolean =
    !file.exists() || (item.modifiedAt ?: 0) * 1000 > file.lastModified() || file.length() != (item.size ?: file.length())
}

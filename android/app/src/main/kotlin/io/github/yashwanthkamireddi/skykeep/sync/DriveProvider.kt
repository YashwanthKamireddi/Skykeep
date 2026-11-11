package io.github.yashwanthkamireddi.skykeep.sync

import android.database.Cursor
import android.database.MatrixCursor
import android.os.CancellationSignal
import android.os.Handler
import android.os.HandlerThread
import android.os.ParcelFileDescriptor
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.provider.DocumentsContract.Root
import android.provider.DocumentsProvider
import io.github.yashwanthkamireddi.skykeep.Engine
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import kotlinx.coroutines.launch
import uniffi.quince_mobile.DItem

/** Pictures asked of Apple in one request: a screenful of a folder and more. */
private const val THUMBS_AT_ONCE = 40

/**
 * iCloud Drive in Android's Files app and in every app's open and save pickers. Folders are read
 * from Apple and kept in the engine's library, so a folder opened before shows at once, then
 * again as Apple has it now.
 *
 * A file opened to read is served as the app reads it, never fetched whole first: through a
 * file descriptor Android makes for us (StorageManager.openProxyFileDescriptor), each read
 * answered from the parts already on the phone or fetched from Apple then, 4 MB at a time, into
 * the desktop folder's own chunk cache (quince-vfs, a bounded share of the cache) — a film plays
 * and a large PDF opens at once. A file whose whole copy is on the phone is opened directly.
 *
 * A file opened to write is held in the engine's journal from that moment (DriveWrites): its
 * bytes in the app's private files and the version they were opened at. It is sent once the
 * app closes it, by WorkManager work that outlives the process: into that same file when Apple
 * still has the version opened, beside it as a conflicted copy when another device changed it
 * meanwhile, never over that device's work. Until it has gone it is the copy every read sees.
 *
 * Images, films, PDFs and documents get Apple's own thumbnails (openDocumentThumbnail).
 */
class DriveProvider : DocumentsProvider() {
  private val engine by lazy { Engine.account(context!!) }
  private val authority by lazy { "${context!!.packageName}.drive" }

  /** Folders read again from Apple in the background, each at most once a minute. */
  private val refreshing = Executors.newFixedThreadPool(2)
  private val readAt = ConcurrentHashMap<String, Long>()
  private val closes by lazy { Handler(HandlerThread("drive-closes").apply { start() }.looper) }
  /** The engine's next chunk of a file read straight on, fetched ahead of the app's reads. */
  private val prefetch = Executors.newFixedThreadPool(2)
  /** One batch of thumbnails asked of Apple per folder at a time; the others wait for it. */
  private val thumbBatches = ConcurrentHashMap<String, java.util.concurrent.FutureTask<Unit>>()

  override fun onCreate() = true

  private val rootColumns = arrayOf(
    Root.COLUMN_ROOT_ID, Root.COLUMN_DOCUMENT_ID, Root.COLUMN_TITLE, Root.COLUMN_ICON, Root.COLUMN_FLAGS, Root.COLUMN_MIME_TYPES,
  )
  private val docColumns = arrayOf(
    Document.COLUMN_DOCUMENT_ID, Document.COLUMN_MIME_TYPE, Document.COLUMN_DISPLAY_NAME,
    Document.COLUMN_LAST_MODIFIED, Document.COLUMN_FLAGS, Document.COLUMN_SIZE,
  )

  override fun queryRoots(projection: Array<out String>?): Cursor {
    val c = MatrixCursor(projection ?: rootColumns)
    // Only when signed in: no root at all otherwise, as an account removed takes its root away.
    if (phoneSignedIn()) {
      c.newRow().apply {
        add(Root.COLUMN_ROOT_ID, "icloud-drive")
        add(Root.COLUMN_DOCUMENT_ID, rootId())
        add(Root.COLUMN_TITLE, "iCloud Drive")
        add(Root.COLUMN_ICON, io.github.yashwanthkamireddi.skykeep.R.mipmap.ic_launcher)
        add(Root.COLUMN_FLAGS, Root.FLAG_SUPPORTS_CREATE or Root.FLAG_SUPPORTS_IS_CHILD or Root.FLAG_SUPPORTS_SEARCH)
        add(Root.COLUMN_MIME_TYPES, "*/*")
      }
      // Saves an earlier run held and never sent (the process ended first) go now.
      DriveWrites.resume(context!!)
    }
    return c
  }

  /** Signed in as the engine sees it: a session kept and an account it belongs to. */
  private fun phoneSignedIn() = runCatching { engine.hasSession() && engine.accountTag() != null }.getOrDefault(false)

  /**
   * The Drive root as other apps see it: named for the account ("root:<tag>"), never the
   * engine's own root id, which is the same for every Apple Account. A folder grant an app was
   * given under one account is checked against that name ([isChildDocument]), so it never
   * reaches the next account's Drive on the same phone. Every other id is Apple's own, unique to
   * its account, and unknown to another account's library.
   */
  private fun rootId(): String = "root:" + (engine.accountTag() ?: throw java.io.FileNotFoundException("signed out"))

  /** An id from another app, as the engine knows it; another account's root is not found. */
  private fun inner(id: String): String = when {
    id == rootId() -> engine.driveRoot()
    id.startsWith("root:") -> throw java.io.FileNotFoundException("not this account's iCloud Drive")
    else -> id
  }

  /** An engine id, as other apps are given it. */
  private fun outer(id: String): String = if (id == engine.driveRoot()) rootId() else id

  private fun mimeOf(item: DItem): String = DriveFiles.mimeOf(item)

  private fun MatrixCursor.add(item: DItem) {
    newRow().apply {
      add(Document.COLUMN_DOCUMENT_ID, outer(item.id))
      add(Document.COLUMN_MIME_TYPE, mimeOf(item))
      add(Document.COLUMN_DISPLAY_NAME, item.name)
      add(Document.COLUMN_LAST_MODIFIED, item.modifiedAt?.let { it * 1000 })
      add(Document.COLUMN_SIZE, item.size)
      add(
        Document.COLUMN_FLAGS,
        if (item.folder) Document.FLAG_DIR_SUPPORTS_CREATE or Document.FLAG_SUPPORTS_DELETE or Document.FLAG_SUPPORTS_RENAME or Document.FLAG_SUPPORTS_MOVE
        else Document.FLAG_SUPPORTS_WRITE or Document.FLAG_SUPPORTS_DELETE or Document.FLAG_SUPPORTS_RENAME or Document.FLAG_SUPPORTS_MOVE or
          (if (DriveFiles.thumbnailable(item)) Document.FLAG_SUPPORTS_THUMBNAIL else 0),
      )
    }
  }

  override fun queryDocument(documentId: String, projection: Array<out String>?): Cursor {
    val c = MatrixCursor(projection ?: docColumns)
    val id = inner(documentId)
    if (id == engine.driveRoot()) {
      c.add(DItem(id, null, "iCloud Drive", true, null, null, null, null))
    } else {
      engine.driveItem(id)?.let { c.add(it) }
    }
    return c
  }

  override fun queryChildDocuments(parentDocumentId: String, projection: Array<out String>?, sortOrder: String?): Cursor {
    val c = MatrixCursor(projection ?: docColumns)
    val parent = inner(parentDocumentId)
    // What the phone has: at once (read from Apple first when never seen).
    runCatching { engine.driveChildren(parent, false) }.getOrNull()?.forEach { c.add(it) }
    val uri = DocumentsContract.buildChildDocumentsUri(authority, parentDocumentId)
    c.setNotificationUri(context!!.contentResolver, uri)
    // Then as Apple has it now, and the Files app told when it is in.
    val now = System.currentTimeMillis()
    if (now - (readAt[parentDocumentId] ?: 0) > 60_000) {
      readAt[parentDocumentId] = now
      refreshing.execute {
        if (runCatching { engine.driveChildren(parent, true) }.isSuccess) {
          context!!.contentResolver.notifyChange(uri, null, false)
        }
      }
    }
    return c
  }

  override fun querySearchDocuments(rootId: String, query: String, projection: Array<out String>?): Cursor {
    val c = MatrixCursor(projection ?: docColumns)
    runCatching { engine.driveSearch(query) }.getOrNull()?.forEach { c.add(it) }
    return c
  }

  /**
   * Whether a document is inside a folder: how Android checks what a folder grant reaches. Both
   * sides compared as other apps name them, so a grant on another account's root ("root:<tag>"
   * of an account signed out since) contains nothing here.
   */
  override fun isChildDocument(parentDocumentId: String, documentId: String): Boolean {
    if (!phoneSignedIn()) return false
    var at: String? = documentId
    var steps = 0
    while (at != null && steps++ < 64) {
      if (at == parentDocumentId) return true
      at = if (at.startsWith("root:")) null else engine.driveItem(at)?.parentId?.let(::outer)
    }
    return false
  }

  /** Where a file's copy is kept on the phone: its own folder per item, named as the file is. */
  private fun copyOf(item: DItem): File = DriveFiles.copyOf(context!!, item)

  override fun openDocument(openedId: String, mode: String, signal: CancellationSignal?): ParcelFileDescriptor = engineCall {
    val documentId = inner(openedId)
    val item = engine.driveItem(documentId) ?: throw java.io.FileNotFoundException("not in iCloud Drive")
    val parsed = ParcelFileDescriptor.parseMode(mode)
    val writing = parsed and (ParcelFileDescriptor.MODE_WRITE_ONLY or ParcelFileDescriptor.MODE_READ_WRITE) != 0
    val file = copyOf(item)
    if (writing) return@engineCall openToWrite(item, file, parsed)
    // A save not yet at Apple is the newest there is: read from where it is held.
    engine.driveWritePath(documentId)?.let { return@engineCall ParcelFileDescriptor.open(File(it), parsed) }
    // The whole file on the phone already (opened in Skykeep's Drive): opened directly.
    if (!DriveFiles.stale(file, item)) return@engineCall ParcelFileDescriptor.open(file, parsed)
    signal?.throwIfCanceled()
    openToRead(documentId)
  }

  /**
   * A file read in parts as the app reads it: a descriptor Android serves by calling [DriveProxy]
   * on a thread of this file's own, ended when the app closes it. Its length is the file's as
   * served (asked of Apple for a package, whose listed size is not its own).
   */
  private fun openToRead(documentId: String): ParcelFileDescriptor {
    val reader = engine.driveReader(documentId)
    val size = try { reader.size().toLong() } catch (e: Exception) { reader.close(); throw e }
    val thread = HandlerThread("drive-read").apply { start() }
    val storage = context!!.getSystemService(android.os.storage.StorageManager::class.java)
    return try {
      storage.openProxyFileDescriptor(
        ParcelFileDescriptor.MODE_READ_ONLY,
        DriveProxy(reader, size, prefetch) { thread.quitSafely() },
        Handler(thread.looper),
      )
    } catch (e: Exception) {
      reader.close(); thread.quitSafely(); throw e
    }
  }

  /**
   * A file opened by another app to write: held in the engine's journal from now, at the version
   * opened (its bytes the phone's copy when that is the version, else Apple's, none when the app
   * writes it whole), and sent once the app closes it (DriveWrites). The journal keeps it in the
   * app's private files, so neither the process ending nor Android emptying the cache loses it.
   */
  private fun openToWrite(item: DItem, copy: File, parsed: Int): ParcelFileDescriptor {
    val truncate = parsed and ParcelFileDescriptor.MODE_TRUNCATE != 0
    val held = File(engine.driveWriteOpen(item.id, copy.takeIf { it.exists() }?.path, truncate))
    val app = context!!.applicationContext
    return try {
      ParcelFileDescriptor.open(held, parsed, closes) { _ ->
        // Closed (even with an error: what was written is what the app left, and is kept).
        runCatching { engine.driveWriteClosed(item.id) }
        Work.scope.launch { DriveWrites.send(app) }
      }
    } catch (e: Exception) {
      runCatching { engine.driveWriteClosed(item.id) }
      throw e
    }
  }

  /**
   * Apple's picture of an image, film, PDF or document, kept by file and version: asked of Apple
   * with the pictures of the other files in its folder not drawn yet, in one request, as the
   * Files app asks for the whole folder's. Where Apple has none and the phone has the file whole,
   * drawn here from it. None at all: the Files app keeps the file's icon.
   */
  override fun openDocumentThumbnail(documentId: String, sizeHint: android.graphics.Point?, signal: CancellationSignal?): android.content.res.AssetFileDescriptor = engineCall {
    val id = inner(documentId)
    val item = engine.driveItem(id)?.takeIf { DriveFiles.thumbnailable(it) } ?: throw java.io.FileNotFoundException("no picture")
    val kept = DriveFiles.thumbnailOf(context!!, item)
    if (!kept.exists()) thumbnailsFor(item)
    signal?.throwIfCanceled()
    if (kept.length() == 0L) drawHere(item, kept, sizeHint, signal)
    if (!kept.exists() || kept.length() == 0L) throw java.io.FileNotFoundException("no picture")
    android.content.res.AssetFileDescriptor(ParcelFileDescriptor.open(kept, ParcelFileDescriptor.MODE_READ_ONLY), 0, kept.length())
  }

  /** The folder's pictures not yet kept, `item`'s among them, asked of Apple once at a time. */
  private fun thumbnailsFor(item: DItem) {
    val folder = item.parentId ?: return
    val mine = java.util.concurrent.FutureTask {
      val wanted = (listOf(item) + runCatching { engine.driveChildren(folder, false) }.getOrDefault(emptyList()))
        .filter { DriveFiles.thumbnailable(it) && !DriveFiles.thumbnailOf(context!!, it).exists() }
        .distinctBy { it.id }.take(THUMBS_AT_ONCE)
      val got = engine.driveThumbnails(wanted.map { it.id }).associate { it.id to it.image }
      for (w in wanted) {
        val at = DriveFiles.thumbnailOf(context!!, w)
        at.parentFile?.let { dir -> dir.mkdirs(); dir.listFiles()?.forEach { if (it != at) it.delete() } }
        // Whole or not at all; empty records that Apple has no picture of this version.
        val part = File(at.path + ".part")
        part.writeBytes(got[w.id] ?: ByteArray(0))
        part.renameTo(at)
      }
    }
    val running = thumbBatches.putIfAbsent(folder, mine)
    if (running != null) { runCatching { running.get() }; return }
    try { mine.run(); mine.get() } finally { thumbBatches.remove(folder, mine) }
  }

  /**
   * A picture drawn from the phone's own whole copy, at the size asked: Android's own
   * thumbnailers for an image or film, the first page for a PDF. Nothing is fetched for it.
   */
  private fun drawHere(item: DItem, kept: File, hint: android.graphics.Point?, signal: CancellationSignal?) {
    val copy = copyOf(item)
    if (DriveFiles.stale(copy, item)) return
    val size = android.util.Size(maxOf(hint?.x ?: 256, 96), maxOf(hint?.y ?: 256, 96))
    val mime = DriveFiles.mimeOf(item)
    val bitmap = runCatching {
      when {
        mime.startsWith("image/") -> android.media.ThumbnailUtils.createImageThumbnail(copy, size, signal)
        mime.startsWith("video/") -> android.media.ThumbnailUtils.createVideoThumbnail(copy, size, signal)
        mime == "application/pdf" -> firstPage(copy, size)
        else -> null
      }
    }.getOrNull() ?: return
    val part = File(kept.path + ".part")
    part.outputStream().use { bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 85, it) }
    part.renameTo(kept)
  }

  private fun firstPage(file: File, size: android.util.Size): android.graphics.Bitmap? =
    ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY).use { fd ->
      android.graphics.pdf.PdfRenderer(fd).use { pdf ->
        if (pdf.pageCount == 0) return null
        pdf.openPage(0).use { page ->
          val scale = minOf(size.width.toFloat() / page.width, size.height.toFloat() / page.height)
          val bitmap = android.graphics.Bitmap.createBitmap(
            maxOf(1, (page.width * scale).toInt()), maxOf(1, (page.height * scale).toInt()), android.graphics.Bitmap.Config.ARGB_8888,
          )
          bitmap.eraseColor(android.graphics.Color.WHITE)
          page.render(bitmap, null, null, android.graphics.pdf.PdfRenderer.Page.RENDER_MODE_FOR_DISPLAY)
          bitmap
        }
      }
    }

  /**
   * An engine call made for another app: a refusal or a fault inside the engine is that app's
   * "file not found" (what a DocumentsProvider may throw), never a crash of Skykeep.
   */
  private inline fun <T> engineCall(call: () -> T): T = try {
    call()
  } catch (e: Exception) {
    if (!e.fromEngine()) throw e
    throw java.io.FileNotFoundException(e.reason("iCloud Drive couldn't open it"))
  }

  /**
   * A new file or folder, named by the app asking. The name is refused, never altered, when it
   * isn't one plain name (a slash, NUL, "." or ".."): it becomes a file on the phone before it
   * goes to Apple, and a name like "../../files/x" must never reach outside its own folder.
   */
  override fun createDocument(parentDocumentId: String, mimeType: String, displayName: String): String {
    val parent = inner(parentDocumentId)
    val name = displayName.trim()
    require(name.isNotEmpty() && name != "." && name != ".." && '/' !in name && '\u0000' !in name) { "Not a file name" }
    if (mimeType == Document.MIME_TYPE_DIR) return outer(engine.driveMkdir(parent, name).id)
    // A new file: made at Apple at once, empty, so it has its id; what the app then writes into
    // it goes into that same file. Staged in a folder of its own, never shared with another.
    val dir = File(context!!.cacheDir, "drive-new/${java.util.UUID.randomUUID()}")
    try {
      val staged = File(dir, DriveFiles.safeName(name)).also { dir.mkdirs(); it.writeBytes(ByteArray(0)) }
      val made = engine.driveWriteBack(parent, staged.path)
      notifyChildren(parent)
      return outer(made.id)
    } finally {
      dir.deleteRecursively()
    }
  }

  override fun deleteDocument(givenId: String) {
    val documentId = inner(givenId)
    val parent = engine.driveItem(documentId)?.parentId
    engine.driveDelete(documentId)
    parent?.let(::notifyChildren)
  }

  override fun renameDocument(givenId: String, displayName: String): String? {
    val documentId = inner(givenId)
    engine.driveRename(documentId, displayName)
    engine.driveItem(documentId)?.parentId?.let(::notifyChildren)
    return null
  }

  override fun moveDocument(sourceDocumentId: String, sourceParentDocumentId: String, targetParentDocumentId: String): String {
    val target = inner(targetParentDocumentId)
    engine.driveMove(inner(sourceDocumentId), target)
    notifyChildren(inner(sourceParentDocumentId)); notifyChildren(target)
    return sourceDocumentId
  }

  /** The Files app told a folder changed; `folder` is the engine's id. */
  private fun notifyChildren(folder: String) =
    context!!.contentResolver.notifyChange(DocumentsContract.buildChildDocumentsUri(authority, outer(folder)), null, false)
}

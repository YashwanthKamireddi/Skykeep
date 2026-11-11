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
import android.webkit.MimeTypeMap
import io.github.yashwanthkamireddi.skykeep.Engine
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import uniffi.quince_mobile.DItem

/**
 * iCloud Drive in Android's Files app and in every app's open and save pickers. Folders are read
 * from Apple and kept in the engine's library, so a folder opened before shows at once, then
 * again as Apple has it now. A file is fetched whole when an app opens it; a file an app wrote is
 * sent back when the app closes it, through the desktop's own write-back (the engine's
 * `drive_write_back`): into that same file when it is still the version opened, beside it as a
 * conflicted copy when it changed on another device meanwhile, never over that device's work.
 */
class DriveProvider : DocumentsProvider() {
  private val engine by lazy { Engine.account(context!!) }
  private val authority by lazy { "${context!!.packageName}.drive" }

  /** Writes sent back one at a time, in order, after the app writing closes the file. */
  private val sending = Executors.newSingleThreadExecutor()
  /** Folders read again from Apple in the background, each at most once a minute. */
  private val refreshing = Executors.newFixedThreadPool(2)
  private val readAt = ConcurrentHashMap<String, Long>()
  /**
   * Files written on this phone and not yet at Apple: their copy here is the newest, so it is
   * never fetched over (10 Nov: a read between a save and its send fetched Apple's older copy
   * over the save, and the send then sent that, losing the save).
   */
  private val unsent: MutableSet<String> = ConcurrentHashMap.newKeySet()
  private val closes by lazy { Handler(HandlerThread("drive-closes").apply { start() }.looper) }

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
        else Document.FLAG_SUPPORTS_WRITE or Document.FLAG_SUPPORTS_DELETE or Document.FLAG_SUPPORTS_RENAME or Document.FLAG_SUPPORTS_MOVE,
      )
    }
  }

  override fun queryDocument(documentId: String, projection: Array<out String>?): Cursor {
    val c = MatrixCursor(projection ?: docColumns)
    val id = inner(documentId)
    if (id == engine.driveRoot()) {
      c.add(DItem(id, null, "iCloud Drive", true, null, null, null))
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

  override fun openDocument(openedId: String, mode: String, signal: CancellationSignal?): ParcelFileDescriptor {
    val documentId = inner(openedId)
    val item = engine.driveItem(documentId) ?: throw java.io.FileNotFoundException("not in iCloud Drive")
    val file = copyOf(item)
    val writing = mode.contains('w') || mode.contains('a')
    // The bytes on the phone, fetched whole when there is no copy, or Apple's is newer.
    val stale = documentId !in unsent &&
      DriveFiles.stale(file, item)
    if (stale && !(writing && mode.contains('t'))) {
      signal?.throwIfCanceled()
      engine.driveDownload(documentId, file.path)
    }
    val parsed = ParcelFileDescriptor.parseMode(mode)
    if (!writing) return ParcelFileDescriptor.open(file, parsed)
    // Written by another app: this copy is the newest from now, and is sent back once the app
    // closes the file. Until Apple has it, nothing fetches over it.
    unsent += documentId
    return ParcelFileDescriptor.open(file, parsed, closes) { error ->
      val parent = item.parentId
      if (error != null || parent == null) return@open
      sending.execute {
        runCatching { engine.driveWriteBack(parent, file.path, documentId) }
          .onSuccess {
            // At Apple now, as this copy: the copy is current, not to be fetched again.
            file.setLastModified(System.currentTimeMillis())
            unsent -= documentId
          }
          .onFailure { android.util.Log.w("Skykeep", "A file written in iCloud Drive wasn't sent yet: ${it.message}") }
        notifyChildren(parent)
      }
    }
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
      val made = engine.driveWriteBack(parent, staged.path, null)
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

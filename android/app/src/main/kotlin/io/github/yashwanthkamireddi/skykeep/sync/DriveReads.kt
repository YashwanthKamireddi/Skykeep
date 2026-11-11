package io.github.yashwanthkamireddi.skykeep.sync

import android.os.ProxyFileDescriptorCallback
import android.system.ErrnoException
import android.system.OsConstants
import java.util.concurrent.Executor

/**
 * A file read as an app reads it, a block at a time, the last few blocks kept in memory: an app
 * reads in small pieces (4 to 128 KB), and each would otherwise cross into the engine on its
 * own. `fetch(offset, length)` answers the bytes from `offset` (fewer only at the end). Reading
 * straight on, the next of the engine's chunks ([CHUNK]) is asked for ahead (`ahead`), so a film
 * plays on while the following part comes. Pure, so it is tested off the phone.
 */
class Blocks(
  private val size: Long,
  private val fetch: (Long, Int) -> ByteArray,
  private val ahead: (Long) -> Unit = {},
  private val block: Int = BLOCK,
  private val most: Int = KEPT,
) {
  /** Kept blocks by their start, oldest use first: at most [most] of them. */
  private val kept = object : LinkedHashMap<Long, ByteArray>(most, 0.75f, true) {
    override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Long, ByteArray>?) = size > most
  }
  private var lastEnd = -1L
  private var askedAhead = -1L

  /** Up to `length` bytes from `offset` into `into`; answers how many (0 at the end). */
  fun read(offset: Long, length: Int, into: ByteArray): Int {
    if (offset >= size || length <= 0) return 0
    val end = minOf(offset + length, size)
    var at = offset
    while (at < end) {
      val start = at - at % block
      val bytes = kept[start] ?: fetch(start, minOf(block.toLong(), size - start).toInt()).also {
        if (it.isEmpty()) throw java.io.EOFException("the file ended early")
        kept[start] = it
      }
      val from = (at - start).toInt()
      val n = minOf(bytes.size - from, (end - at).toInt())
      if (n <= 0) break
      System.arraycopy(bytes, from, into, (at - offset).toInt(), n)
      at += n
    }
    // Straight on from the last read: the chunk after this one, asked for once.
    if (offset == lastEnd) {
      val next = (at / CHUNK + 1) * CHUNK
      if (next < size && next != askedAhead) { askedAhead = next; ahead(next) }
    }
    lastEnd = at
    return (at - offset).toInt()
  }

  companion object {
    /** What is fetched from the engine at a time. */
    const val BLOCK = 256 * 1024

    /** Blocks kept per open file: 2 MB. */
    const val KEPT = 8

    /** The engine's chunk, fetched from Apple at a time (quince_vfs::CHUNK). */
    const val CHUNK = 4L * 1024 * 1024
  }
}

/**
 * A Drive file served to another app through a file descriptor Android makes for it
 * (StorageManager.openProxyFileDescriptor): every read the app makes comes here, on this file's
 * own thread, and is served from [Blocks] over the engine's reader. A part Apple can't send is
 * an I/O error for that read, never the end of the app.
 */
class DriveProxy(
  private val reader: uniffi.quince_mobile.DriveReader,
  private val size: Long,
  prefetch: Executor,
  private val done: () -> Unit,
) : ProxyFileDescriptorCallback() {
  private val blocks = Blocks(
    size,
    fetch = { offset, length -> reader.read(offset.toULong(), length.toUInt()) },
    ahead = { at -> prefetch.execute { runCatching { reader.read(at.toULong(), 1u) } } },
  )

  override fun onGetSize() = size

  override fun onRead(offset: Long, size: Int, data: ByteArray): Int = try {
    blocks.read(offset, size, data)
  } catch (e: Exception) {
    android.util.Log.w("Skykeep", "A part of a Drive file couldn't be read: ${e.reason(e.javaClass.simpleName)}")
    throw ErrnoException("onRead", OsConstants.EIO)
  }

  override fun onRelease() {
    runCatching { reader.close() }
    done()
  }
}

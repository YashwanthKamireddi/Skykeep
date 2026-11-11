package io.github.yashwanthkamireddi.skykeep.sync

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** A Drive file read in parts for another app (DriveProxy), against a file held in memory. */
class BlocksTest {
  private val file = ByteArray(10 * 1024 * 1024 + 123) { (it % 251).toByte() }
  private val fetched = mutableListOf<Pair<Long, Int>>()
  private val ahead = mutableListOf<Long>()

  private fun blocks(most: Int = Blocks.KEPT) = Blocks(
    file.size.toLong(),
    fetch = { offset, length ->
      fetched += offset to length
      file.copyOfRange(offset.toInt(), minOf(file.size, offset.toInt() + length))
    },
    ahead = { ahead += it },
    most = most,
  )

  /** A read across two blocks gives exactly the bytes asked for; read again, nothing is fetched. */
  @Test
  fun aReadAcrossBlocksIsExactAndKept() {
    val b = blocks()
    val into = ByteArray(1000)
    val start = Blocks.BLOCK - 300L
    assertEquals(1000, b.read(start, 1000, into))
    assertArrayEquals(file.copyOfRange(start.toInt(), start.toInt() + 1000), into)
    assertEquals(listOf(0L to Blocks.BLOCK, Blocks.BLOCK.toLong() to Blocks.BLOCK), fetched)
    b.read(start, 1000, into)
    assertEquals("served from memory", 2, fetched.size)
  }

  /** The end of the file: what there is, then nothing — never a read past it. */
  @Test
  fun theEndGivesWhatThereIsThenNothing() {
    val b = blocks()
    val into = ByteArray(4096)
    val tail = file.size - 100L
    assertEquals(100, b.read(tail, 4096, into))
    assertArrayEquals(file.copyOfRange(tail.toInt(), file.size), into.copyOf(100))
    assertEquals(0, b.read(file.size.toLong(), 4096, into))
    assertTrue("the last block asked at its own length", fetched.all { it.first + it.second <= file.size })
  }

  /** Memory stays bounded: a long read keeps only the last few blocks. */
  @Test
  fun onlyTheLastFewBlocksAreKept() {
    val b = blocks(most = 2)
    val into = ByteArray(Blocks.BLOCK)
    for (i in 0 until 4) b.read(i * Blocks.BLOCK.toLong(), Blocks.BLOCK, into)
    fetched.clear()
    b.read(0, 10, into)
    assertEquals("the first block was let go of", listOf(0L to Blocks.BLOCK), fetched)
  }

  /** Read straight on, the next of the engine's chunks is asked for ahead, once; a jump is not. */
  @Test
  fun readingStraightOnAsksForTheNextChunkOnce() {
    val b = blocks()
    val into = ByteArray(64 * 1024)
    b.read(0, into.size, into)
    assertTrue("a first read is no pattern yet", ahead.isEmpty())
    b.read(into.size.toLong(), into.size, into)
    b.read(2L * into.size, into.size, into)
    assertEquals(listOf(Blocks.CHUNK), ahead)
    b.read(9L * 1024 * 1024, into.size, into)
    assertEquals("a seek is not reading on", 1, ahead.size)
  }
}

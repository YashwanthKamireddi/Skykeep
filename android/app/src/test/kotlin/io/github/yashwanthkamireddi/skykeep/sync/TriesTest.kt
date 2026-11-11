package io.github.yashwanthkamireddi.skykeep.sync

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

/** Photo backup's count of failed tries, kept across runs in one preference. */
class TriesTest {
  private val key = "phone|1a2b|content://media/external/images/media/42"

  /** What is kept is read back the same, keys with ':' and '|' and all. */
  @Test
  fun aCountRoundTrips() {
    val tries = mutableMapOf(key to 2, "$key/2" to 4)
    assertEquals(tries, Tries.decode(Tries.encode(tries)))
    assertEquals(emptyMap<String, Int>(), Tries.decode(""))
  }

  /** A damaged entry is passed over, never a crash of the backup. */
  @Test
  fun aDamagedEntryIsPassedOver() {
    assertEquals(mapOf(key to 1), Tries.decode("$key=1,nonsense,other=x"))
  }

  /** After the last try the photo is let go of (it stays on the phone). */
  @Test
  fun theLastTryLetsGo() {
    val tries = mutableMapOf<String, Int>()
    repeat(Tries.MOST - 1) { assertEquals(it + 1, Tries.failed(tries, key)) }
    assertEquals(Tries.MOST, Tries.failed(tries, key))
    assertFalse(key in tries)
  }
}

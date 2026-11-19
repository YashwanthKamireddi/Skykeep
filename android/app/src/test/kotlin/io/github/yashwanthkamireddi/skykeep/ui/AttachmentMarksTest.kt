package io.github.yashwanthkamireddi.skykeep.ui

import androidx.compose.ui.text.AnnotatedString
import org.junit.Assert.assertEquals
import org.junit.Test

/** The note editor's 📎 for an attachment (discussion #199), and its cursor kept exact. */
class AttachmentMarksTest {
  private val note = "Trip\n￼\nTickets ￼ end"

  /** Each attachment shows as 📎, never the font's "OBJ" box; the text saved is untouched. */
  @Test
  fun anAttachmentShowsAsAPaperclip() {
    val shown = AttachmentMarks.filter(AnnotatedString(note)).text.text
    assertEquals("Trip\n📎\nTickets 📎 end", shown)
  }

  /** Every place in the note maps to its place in what is shown and back again, and a place
   *  inside a 📎 (two characters wide) falls just before its attachment. */
  @Test
  fun everyCursorPlaceMapsBothWays() {
    val map = AttachmentMarks.filter(AnnotatedString(note)).offsetMapping
    for (i in 0..note.length) assertEquals("place $i", i, map.transformedToOriginal(map.originalToTransformed(i)))
    val first = note.indexOf('￼')
    assertEquals(first, map.transformedToOriginal(map.originalToTransformed(first) + 1))
    val second = note.lastIndexOf('￼')
    assertEquals(second, map.transformedToOriginal(map.originalToTransformed(second) + 1))
  }

  /** A note with no attachment is shown exactly as it is. */
  @Test
  fun aPlainNoteIsUntouched() {
    val plain = AttachmentMarks.filter(AnnotatedString("Groceries"))
    assertEquals("Groceries", plain.text.text)
    assertEquals(4, plain.offsetMapping.originalToTransformed(4))
  }
}

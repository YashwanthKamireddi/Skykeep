package io.github.yashwanthkamireddi.skykeep.ui.theme

import kotlin.math.pow
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Every colour pair Skykeep draws text or meaning with, held to WCAG 2.2 AA: 4.5:1 for text,
 * 3:1 for icons and graphics. A new or changed token that fails is caught here, not on a
 * phone. (The brand's own traps: white on sky is 2.9:1, sky on white too.)
 */
class PaletteContrastTest {
  private fun channel(v: Long, shift: Int): Double {
    val s = ((v shr shift) and 0xFF) / 255.0
    return if (s <= 0.04045) s / 12.92 else ((s + 0.055) / 1.055).pow(2.4)
  }
  private fun luminance(v: Long) = 0.2126 * channel(v, 16) + 0.7152 * channel(v, 8) + 0.0722 * channel(v, 0)
  private fun ratio(a: Long, b: Long): Double {
    val (hi, lo) = listOf(luminance(a), luminance(b)).sortedDescending()
    return (hi + 0.05) / (lo + 0.05)
  }

  private val failures = mutableListOf<String>()
  private fun need(min: Double, what: String, fg: Long, bg: Long) {
    val r = ratio(fg, bg)
    if (r < min) failures += "%s: %.2f:1 (needs %.1f) #%06X on #%06X".format(what, r, min, fg and 0xFFFFFF, bg and 0xFFFFFF)
  }

  private fun scheme(name: String, p: Palette.Roles) {
    val surfaces = mapOf(
      "background" to p.background, "surface" to p.surface, "containerLowest" to p.surfaceContainerLowest,
      "containerLow" to p.surfaceContainerLow, "container" to p.surfaceContainer,
      "containerHigh" to p.surfaceContainerHigh, "containerHighest" to p.surfaceContainerHighest,
    )
    for ((s, bg) in surfaces) {
      need(4.5, "$name onSurface on $s", p.onSurface, bg)
      need(4.5, "$name onSurfaceVariant on $s", p.onSurfaceVariant, bg)
      need(4.5, "$name primary (text, links) on $s", p.primary, bg)
      need(4.5, "$name error (text) on $s", p.error, bg)
      need(3.0, "$name outline on $s", p.outline, bg)
    }
    need(4.5, "$name onPrimary on primary", p.onPrimary, p.primary)
    need(4.5, "$name onPrimaryContainer on primaryContainer", p.onPrimaryContainer, p.primaryContainer)
    need(4.5, "$name onSecondary on secondary", p.onSecondary, p.secondary)
    need(4.5, "$name onSecondaryContainer on secondaryContainer", p.onSecondaryContainer, p.secondaryContainer)
    need(4.5, "$name onError on error", p.onError, p.error)
    need(4.5, "$name onErrorContainer on errorContainer", p.onErrorContainer, p.errorContainer)
    need(4.5, "$name inverseOnSurface on inverseSurface", p.inverseOnSurface, p.inverseSurface)
    for ((s, bg) in surfaces) need(3.0, "$name accent (icons) on $s", p.accent, bg)
    for ((label, colour) in Palette.Smart.run {
      listOf("today" to TODAY, "scheduled" to SCHEDULED, "all" to ALL, "flagged" to FLAGGED, "completed" to COMPLETED)
    }) {
      need(3.0, "smart $label circle under its white glyph", Palette.WHITE, colour)
      need(3.0, "$name smart $label circle on its card", colour, p.surfaceContainer)
    }
    val dark = name == "dark"
    for (app in Palette.App.entries) {
      val mark = if (dark) app.dark else app.light
      need(3.0, "$name ${app.name} mark on the background", mark, p.background)
      need(3.0, "$name ${app.name} mark on a card", mark, p.surfaceContainer)
      Palette.accent(app, dark)?.let { a ->
        for ((s, bg) in surfaces) need(4.5, "$name ${app.name} primary (text) on $s", a.primary, bg)
        need(4.5, "$name ${app.name} onPrimary on primary", a.onPrimary, a.primary)
        need(4.5, "$name ${app.name} onPrimaryContainer on primaryContainer", a.onPrimaryContainer, a.primaryContainer)
      }
    }
  }

  @Test
  fun every_pair_meets_its_contrast() {
    scheme("light", Palette.light)
    scheme("dark", Palette.dark)
    assertTrue(failures.joinToString("\n", prefix = "\n"), failures.isEmpty())
  }

  @Test
  fun the_brand_traps_stay_traps() {
    // If these ever pass, the reasoning in Palette's comment is wrong and must be revisited.
    assertTrue(ratio(Palette.WHITE, Palette.SKY) < 3.0)
    assertTrue(ratio(Palette.WHITE, Palette.BLUE) >= 4.5)
    assertTrue(ratio(Palette.INK, Palette.SKY) >= 4.5)
  }
}

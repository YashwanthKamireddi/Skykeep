package io.github.yashwanthkamireddi.skykeep.ui.theme

import androidx.compose.material3.Typography
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp

private fun style(size: Int, line: Int, weight: FontWeight = FontWeight.Normal, tracking: Double = 0.0) =
  TextStyle(fontSize = size.sp, lineHeight = line.sp, fontWeight = weight, letterSpacing = tracking.em)

/**
 * The device's own sans (no bundled font: size, licence, and it scales with the person's font
 * setting). Regular and Medium only; every line height at least 1.25 times its size, so text
 * still reads at a 200% font scale.
 *
 * headlineLarge 28/34 the Welcome headline · headlineMedium 24/30 a screen's title ·
 * headlineSmall 22/28 big numbers (counts) · titleLarge 20/26, titleMedium 16/22 card titles ·
 * titleSmall 14/20 Medium · bodyLarge 16/24 text · bodyMedium 14/20 secondary text ·
 * bodySmall/labelSmall 12/16 times and captions only, never a status.
 */
val Typography = Typography(
  displayLarge = style(40, 48, FontWeight.Medium),
  displayMedium = style(34, 42, FontWeight.Medium),
  displaySmall = style(30, 38, FontWeight.Medium),
  headlineLarge = style(28, 34, FontWeight.Medium),
  headlineMedium = style(24, 30, FontWeight.Medium),
  headlineSmall = style(22, 28, FontWeight.Medium),
  titleLarge = style(20, 26, FontWeight.Medium),
  titleMedium = style(16, 22, FontWeight.Medium, 0.005),
  titleSmall = style(14, 20, FontWeight.Medium, 0.005),
  bodyLarge = style(16, 24),
  bodyMedium = style(14, 20, tracking = 0.01),
  bodySmall = style(12, 16, tracking = 0.02),
  labelLarge = style(14, 20, FontWeight.Medium, 0.005),
  labelMedium = style(12, 16, FontWeight.Medium, 0.03),
  labelSmall = style(12, 16, FontWeight.Medium, 0.03),
)

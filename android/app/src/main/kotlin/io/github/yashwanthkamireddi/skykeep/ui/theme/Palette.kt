package io.github.yashwanthkamireddi.skykeep.ui.theme

/**
 * Skykeep's colours as plain ARGB numbers: the brand (sky #2F9BFF, ink #151A22, off-white
 * #EEF2F7) laid out as Material 3 roles for light and dark. Plain numbers so a JVM test can
 * check every text pair against WCAG contrast (PaletteContrastTest) without Android.
 *
 * Two traps the numbers rule out: white on sky is 2.9:1, so sky is never a fill under white
 * text (light primary is #0068D6, 5.3:1); and sky as text on white is 2.9:1, so in light mode
 * sky is only an icon or graphic colour. In dark mode sky is the primary, with ink on it.
 */
object Palette {
  const val SKY = 0xFF2F9BFF
  const val BLUE = 0xFF0068D6
  const val INK = 0xFF151A22
  const val OFF_WHITE = 0xFFEEF2F7
  const val WHITE = 0xFFFFFFFF

  /** One set of roles; the names are Material 3's. */
  data class Roles(
    val primary: Long, val onPrimary: Long,
    val primaryContainer: Long, val onPrimaryContainer: Long,
    val secondary: Long, val onSecondary: Long,
    val secondaryContainer: Long, val onSecondaryContainer: Long,
    val background: Long, val onBackground: Long,
    val surface: Long, val onSurface: Long,
    val surfaceVariant: Long, val onSurfaceVariant: Long,
    val surfaceContainerLowest: Long, val surfaceContainerLow: Long, val surfaceContainer: Long,
    val surfaceContainerHigh: Long, val surfaceContainerHighest: Long,
    val outline: Long, val outlineVariant: Long,
    val error: Long, val onError: Long, val errorContainer: Long, val onErrorContainer: Long,
    val inverseSurface: Long, val inverseOnSurface: Long, val inversePrimary: Long,
    /** Brand sky for icons and graphics (never body text in light mode). */
    val accent: Long,
  )

  val light = Roles(
    primary = BLUE, onPrimary = WHITE,
    primaryContainer = 0xFFD7E6FA, onPrimaryContainer = 0xFF002E5F,
    secondary = 0xFF4E5B6C, onSecondary = WHITE,
    secondaryContainer = 0xFFDDE4EE, onSecondaryContainer = INK,
    background = OFF_WHITE, onBackground = INK,
    surface = OFF_WHITE, onSurface = INK,
    surfaceVariant = 0xFFDDE3EB, onSurfaceVariant = 0xFF475161,
    surfaceContainerLowest = WHITE, surfaceContainerLow = 0xFFF7F9FC, surfaceContainer = WHITE,
    surfaceContainerHigh = 0xFFEDF1F5, surfaceContainerHighest = 0xFFE8EDF2,
    outline = 0xFF6F7987, outlineVariant = 0xFFC8D0DA,
    error = 0xFFB3261E, onError = WHITE, errorContainer = 0xFFF9DEDC, onErrorContainer = 0xFF410E0B,
    inverseSurface = 0xFF2A313B, inverseOnSurface = OFF_WHITE, inversePrimary = SKY,
    accent = SKY,
  )

  val dark = Roles(
    primary = SKY, onPrimary = INK,
    primaryContainer = BLUE, onPrimaryContainer = WHITE,
    secondary = 0xFFB6C3D3, onSecondary = 0xFF202833,
    secondaryContainer = 0xFF2D3643, onSecondaryContainer = 0xFFDCE4EE,
    background = INK, onBackground = OFF_WHITE,
    surface = INK, onSurface = OFF_WHITE,
    surfaceVariant = 0xFF2C3440, onSurfaceVariant = 0xFFB4BDC9,
    surfaceContainerLowest = 0xFF10141A, surfaceContainerLow = 0xFF1A1F28, surfaceContainer = 0xFF1E242E,
    surfaceContainerHigh = 0xFF242B35, surfaceContainerHighest = 0xFF29303A,
    outline = 0xFF8A93A0, outlineVariant = 0xFF3B4450,
    error = 0xFFF2B8B5, onError = 0xFF601410, errorContainer = 0xFF8C1D18, onErrorContainer = 0xFFF9DEDC,
    inverseSurface = OFF_WHITE, inverseOnSurface = INK, inversePrimary = BLUE,
    accent = SKY,
  )

  /**
   * Reminders' smart lists, by meaning (the structure people know from their iPhone, in
   * Skykeep's own colours): each a filled circle carrying a white glyph, so each must hold
   * 3:1 against white (the glyph) and against the card it sits on, in light and dark alike.
   */
  object Smart {
    const val TODAY = 0xFF2A82E0
    const val SCHEDULED = 0xFFD84A3F
    const val ALL = 0xFF7A8796
    const val FLAGGED = 0xFFB86E00
    const val COMPLETED = 0xFF778392
  }

  /** The Notes accent: a deep yellow for its icon chip and the editor's caret, never text. */
  const val NOTES = 0xFFE5A800
}

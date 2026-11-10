package io.github.yashwanthkamireddi.skykeep.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

private fun Palette.Roles.scheme(dark: Boolean): ColorScheme {
  fun c(v: Long) = Color(v)
  // Skykeep has no third hue: tertiary takes the secondary roles. No surface tint, so tonal
  // elevation never washes a surface blue. Bright and dim: the plain surface.
  val s = if (dark) darkColorScheme() else lightColorScheme()
  return s.copy(
    primary = c(primary), onPrimary = c(onPrimary),
    primaryContainer = c(primaryContainer), onPrimaryContainer = c(onPrimaryContainer),
    inversePrimary = c(inversePrimary),
    secondary = c(secondary), onSecondary = c(onSecondary),
    secondaryContainer = c(secondaryContainer), onSecondaryContainer = c(onSecondaryContainer),
    tertiary = c(secondary), onTertiary = c(onSecondary),
    tertiaryContainer = c(secondaryContainer), onTertiaryContainer = c(onSecondaryContainer),
    background = c(background), onBackground = c(onBackground),
    surface = c(surface), onSurface = c(onSurface),
    surfaceVariant = c(surfaceVariant), onSurfaceVariant = c(onSurfaceVariant),
    surfaceTint = c(surface),
    inverseSurface = c(inverseSurface), inverseOnSurface = c(inverseOnSurface),
    error = c(error), onError = c(onError), errorContainer = c(errorContainer), onErrorContainer = c(onErrorContainer),
    outline = c(outline), outlineVariant = c(outlineVariant), scrim = Color.Black,
    surfaceBright = c(surface), surfaceDim = c(surface),
    surfaceContainerLowest = c(surfaceContainerLowest), surfaceContainerLow = c(surfaceContainerLow),
    surfaceContainer = c(surfaceContainer), surfaceContainerHigh = c(surfaceContainerHigh),
    surfaceContainerHighest = c(surfaceContainerHighest),
  )
}

/** The brand sky, for icons and graphics: `SkykeepTheme.accent`. */
val LocalAccent = staticCompositionLocalOf { Color(Palette.SKY) }

/** Rounded but quiet: 8 small, 12 cards and fields, 16 sheets, 28 the largest surfaces. */
private val SkykeepShapes = Shapes(
  extraSmall = RoundedCornerShape(6.dp),
  small = RoundedCornerShape(8.dp),
  medium = RoundedCornerShape(12.dp),
  large = RoundedCornerShape(16.dp),
  extraLarge = RoundedCornerShape(28.dp),
)

/**
 * Skykeep's look: Material 3 underneath (Android's own navigation, sheets, switches and back),
 * the fixed brand palette on top, the same on every phone so a status colour always means the
 * same thing. Material 3 1.4's default motion is its standard scheme, the calm one meant for
 * utilities; the one expressive moment (a reminder ticked) uses Motion.tick.
 */
@Composable
fun SkykeepTheme(dark: Boolean = isSystemInDarkTheme(), content: @Composable () -> Unit) {
  val roles = if (dark) Palette.dark else Palette.light
  CompositionLocalProvider(LocalAccent provides Color(roles.accent)) {
    MaterialTheme(colorScheme = roles.scheme(dark), shapes = SkykeepShapes, typography = Typography, content = content)
  }
}

/** An app's own colour (its mark), for the current light or dark. */
@Composable
fun mark(app: Palette.App): Color = Color(if (isSystemInDarkTheme()) app.dark else app.light)

/**
 * An app's screens in its own theme, as on the desktop: Notes in yellow, Calendar in red, the
 * rest in Skykeep's blue. Only the accent roles change; surfaces and text stay the same.
 */
@Composable
fun AppTheme(app: Palette.App, content: @Composable () -> Unit) {
  val accent = Palette.accent(app, isSystemInDarkTheme()) ?: return content()
  val scheme = MaterialTheme.colorScheme.copy(
    primary = Color(accent.primary), onPrimary = Color(accent.onPrimary),
    primaryContainer = Color(accent.primaryContainer), onPrimaryContainer = Color(accent.onPrimaryContainer),
  )
  MaterialTheme(colorScheme = scheme, shapes = MaterialTheme.shapes, typography = MaterialTheme.typography, content = content)
}

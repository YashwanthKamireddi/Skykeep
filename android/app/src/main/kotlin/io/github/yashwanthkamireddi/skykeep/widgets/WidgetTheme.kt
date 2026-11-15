package io.github.yashwanthkamireddi.skykeep.widgets

import android.os.Build
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.glance.GlanceTheme
import androidx.glance.material3.ColorProviders
import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import io.github.yashwanthkamireddi.skykeep.ui.theme.toColorScheme

private val FallbackColors = ColorProviders(
  light = Palette.light.toColorScheme(dark = false),
  dark = Palette.dark.toColorScheme(dark = true),
)

/**
 * Widget theme following Android 15 Material 3 guidelines: uses dynamic wallpaper colors on
 * Android 12+ (`GlanceTheme` platform defaults) and Skykeep's own WCAG-checked palette below
 * API 31.
 */
@Composable
internal fun SkykeepWidgetTheme(content: @Composable () -> Unit) {
  if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
    GlanceTheme(content = content)
  } else {
    GlanceTheme(colors = FallbackColors, content = content)
  }
}

internal fun parseHexColour(hex: String?, fallback: Color = Color(Palette.Smart.TODAY)): Color =
  hex?.removePrefix("#")?.takeIf { it.length == 6 }?.toLongOrNull(16)
    ?.let { Color(0xFF000000L or it) } ?: fallback

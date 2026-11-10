package io.github.yashwanthkamireddi.skykeep.ui.theme

import android.provider.Settings
import androidx.compose.animation.core.AnimationSpec
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext

/**
 * The few motions Skykeep has, each answering an action (none for opening or closing a screen
 * beyond the system's own: those flickered on the desktop). Material 3's spring tokens:
 * stiffness / damping ratio.
 */
object Motion {
  /** Standard fast spatial 1400 / 0.9: a press, a row settling. */
  fun <T> fast(): AnimationSpec<T> = spring(dampingRatio = 0.9f, stiffness = 1400f)
  /** Standard default spatial 700 / 0.9: a sheet or panel settling. */
  fun <T> settle(): AnimationSpec<T> = spring(dampingRatio = 0.9f, stiffness = 700f)
  /** Fast effects 3800 / 1.0 (about 150 ms): colour and opacity, never overshooting. */
  fun <T> effect(): AnimationSpec<T> = spring(dampingRatio = 1f, stiffness = 3800f)
  /** Expressive fast spatial 800 / 0.6: the reminder tick, and nothing else. */
  fun <T> tick(): AnimationSpec<T> = spring(dampingRatio = 0.6f, stiffness = 800f)
}

/** Animations off in the system (Remove animations): every motion becomes an instant change. */
@Composable
fun reducedMotion(): Boolean {
  val resolver = LocalContext.current.contentResolver
  return remember { Settings.Global.getFloat(resolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f }
}

/** [spec], or an instant change when the person has turned animations off. */
@Composable
fun <T> motion(spec: AnimationSpec<T>): AnimationSpec<T> = if (reducedMotion()) snap() else spec

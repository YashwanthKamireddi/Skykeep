package io.github.yashwanthkamireddi.skykeep.ui

import android.content.Context
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette

/** A glyph that stays readable on [fill]: white where white holds 3:1, ink where it doesn't. */
fun glyphOn(fill: Color): Color =
  if ((1.05f / (fill.luminance() + 0.05f)) >= 3f) Color.White else Color(Palette.INK)

/** A service's mark: its icon on a filled rounded square in the service's colour. */
@Composable
fun ServiceChip(icon: Int, colour: Color, size: Int = 40) {
  Box(
    Modifier.size(size.dp).background(colour, MaterialTheme.shapes.medium),
    contentAlignment = Alignment.Center,
  ) { Icon(painterResource(icon), contentDescription = null, tint = glyphOn(colour), modifier = Modifier.size((size * 0.55f).dp)) }
}

/**
 * Something the person should know or do, on Home: an icon, what happened and what is safe in
 * words, and at most two actions. Never colour alone: a [warning] carries the warning icon.
 */
@Composable
fun Notice(text: String, warning: Boolean = false, actions: @Composable () -> Unit = {}) {
  val colours = MaterialTheme.colorScheme
  Surface(
    color = if (warning) colours.errorContainer else colours.secondaryContainer,
    contentColor = if (warning) colours.onErrorContainer else colours.onSecondaryContainer,
    shape = MaterialTheme.shapes.large,
    modifier = Modifier.fillMaxWidth(),
  ) {
    Row(Modifier.padding(16.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
      Icon(painterResource(if (warning) io.github.yashwanthkamireddi.skykeep.R.drawable.ic_warning else io.github.yashwanthkamireddi.skykeep.R.drawable.ic_sync), contentDescription = null, modifier = Modifier.size(20.dp))
      Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(text, style = MaterialTheme.typography.bodyMedium)
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) { actions() }
      }
    }
  }
}

/** Whether the phone can reach the internet now, kept current by Android's network callback. */
@Composable
fun rememberOnline(): Boolean {
  val context = LocalContext.current
  val cm = remember { context.getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager }
  fun now() = cm.getNetworkCapabilities(cm.activeNetwork)?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true
  var online by remember { mutableStateOf(now()) }
  DisposableEffect(cm) {
    val callback = object : ConnectivityManager.NetworkCallback() {
      override fun onAvailable(network: Network) { online = true }
      override fun onLost(network: Network) { online = now() }
      override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
        online = caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
      }
    }
    cm.registerDefaultNetworkCallback(callback)
    onDispose { cm.unregisterNetworkCallback(callback) }
  }
  return online
}

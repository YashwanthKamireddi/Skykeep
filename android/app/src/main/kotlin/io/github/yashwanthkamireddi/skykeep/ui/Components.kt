package io.github.yashwanthkamireddi.skykeep.ui

import androidx.compose.foundation.lazy.itemsIndexed

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
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics

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

/** The desktop's search field: a quiet filled pill with the search mark, no outline. */
@Composable
fun SearchPill(value: String, onChange: (String) -> Unit, modifier: Modifier = Modifier, cursor: Color = MaterialTheme.colorScheme.primary) {
  Surface(color = MaterialTheme.colorScheme.surfaceContainerHigh, shape = MaterialTheme.shapes.medium, modifier = modifier.fillMaxWidth()) {
    Row(Modifier.padding(horizontal = 12.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
      Icon(painterResource(io.github.yashwanthkamireddi.skykeep.R.drawable.ic_search), contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(18.dp))
      androidx.compose.foundation.layout.Spacer(Modifier.size(10.dp))
      androidx.compose.foundation.text.BasicTextField(
        value = value, onValueChange = onChange, singleLine = true,
        textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
        cursorBrush = androidx.compose.ui.graphics.SolidColor(cursor),
        modifier = Modifier.weight(1f).androidx_semanticsSearch(),
        decorationBox = { field ->
          if (value.isEmpty()) Text("Search", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
          field()
        },
      )
    }
  }
}

private fun Modifier.androidx_semanticsSearch() = this.then(Modifier.semantics { contentDescription = "Search" })

/** Rows in one rounded card, divided by hairlines set in past their leading mark, as the desktop groups them. */
@Composable
fun <T> GroupCard(items: List<T>, inset: Int = 56, row: @Composable (T) -> Unit) {
  Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large, modifier = Modifier.fillMaxWidth()) {
    Column {
      items.forEachIndexed { i, item ->
        row(item)
        if (i < items.lastIndex) androidx.compose.material3.HorizontalDivider(Modifier.padding(start = inset.dp), color = MaterialTheme.colorScheme.outlineVariant)
      }
    }
  }
}

/**
 * [GroupCard] for a long list: each row its own item of the lazy list, drawn as its slice of the
 * one card (the first row with the card's top corners, the last with its bottom ones), so only
 * the rows on screen are built. A folder of thousands of notes built every row before its first
 * frame inside one card.
 */
fun <T> androidx.compose.foundation.lazy.LazyListScope.groupItems(
  items: List<T>,
  key: (T) -> Any,
  inset: Int = 56,
  row: @Composable (T) -> Unit,
) {
  itemsIndexed(items, key = { _, item -> key(item) }) { i, item ->
    val card = MaterialTheme.shapes.large
    val none = androidx.compose.foundation.shape.ZeroCornerSize
    val shape = when {
      items.size == 1 -> card
      i == 0 -> card.copy(bottomStart = none, bottomEnd = none)
      i == items.lastIndex -> card.copy(topStart = none, topEnd = none)
      else -> androidx.compose.ui.graphics.RectangleShape
    }
    Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = shape, modifier = Modifier.fillMaxWidth()) {
      Column {
        row(item)
        if (i < items.lastIndex) androidx.compose.material3.HorizontalDivider(Modifier.padding(start = inset.dp), color = MaterialTheme.colorScheme.outlineVariant)
      }
    }
  }
}

/** A section's name above its card. */
@Composable
fun SectionLabel(text: String, modifier: Modifier = Modifier) {
  Text(text, style = MaterialTheme.typography.titleMedium, modifier = modifier.padding(start = 4.dp, top = 12.dp, bottom = 8.dp))
}

/**
 * Whether a screen is showing something full screen (the photo viewer): the bottom bar steps
 * aside while it does. A screen sets it with [FullScreen].
 */
val LocalFullScreen = androidx.compose.runtime.staticCompositionLocalOf { androidx.compose.runtime.mutableStateOf(false) }

/** Marks the caller as full screen for as long as it is shown. */
@Composable
fun FullScreen() {
  val full = LocalFullScreen.current
  DisposableEffect(Unit) {
    full.value = true
    onDispose { full.value = false }
  }
}

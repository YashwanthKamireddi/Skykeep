package io.github.yashwanthkamireddi.skykeep.widgets

import android.appwidget.AppWidgetManager
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.glance.appwidget.GlanceAppWidgetManager
import androidx.glance.appwidget.state.updateAppWidgetState
import io.github.yashwanthkamireddi.skykeep.Engine
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.ui.SectionLabel
import io.github.yashwanthkamireddi.skykeep.ui.glyphOn
import io.github.yashwanthkamireddi.skykeep.ui.theme.SkykeepTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.RCounts
import uniffi.quince_mobile.RList

/**
 * Configuration screen for [RemindersWidget]: lets the person choose which Smart List (All,
 * Today, Scheduled, Flagged) or custom iCloud list that widget instance shows.
 */
class RemindersWidgetConfigActivity : ComponentActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    enableEdgeToEdge()
    setResult(RESULT_CANCELED)

    val appWidgetId = intent?.extras?.getInt(
      AppWidgetManager.EXTRA_APPWIDGET_ID,
      AppWidgetManager.INVALID_APPWIDGET_ID,
    ) ?: AppWidgetManager.INVALID_APPWIDGET_ID

    if (appWidgetId == AppWidgetManager.INVALID_APPWIDGET_ID) {
      finish()
      return
    }

    setContent {
      SkykeepTheme {
        RemindersConfigScreen(
          onCancel = { finish() },
          onPick = { viewKind, listGuid ->
            val resultIntent = Intent().putExtra(AppWidgetManager.EXTRA_APPWIDGET_ID, appWidgetId)
            val app = applicationContext
            kotlinx.coroutines.MainScope().launch {
              val glanceId = GlanceAppWidgetManager(app).getGlanceIdBy(appWidgetId)
              updateAppWidgetState(app, glanceId) { prefs ->
                prefs[WidgetPrefs.VIEW_KIND] = viewKind
                if (listGuid != null) prefs[WidgetPrefs.LIST_GUID] = listGuid
                else prefs.remove(WidgetPrefs.LIST_GUID)
                prefs[WidgetPrefs.REVISION] = System.nanoTime()
              }
              WidgetUpdates.bumpRemindersTick()
              RemindersWidget().update(app, glanceId)
              setResult(RESULT_OK, resultIntent)
              finish()
            }
          },
        )
      }
    }
  }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RemindersConfigScreen(
  onCancel: () -> Unit,
  onPick: (viewKind: String, listGuid: String?) -> Unit,
) {
  val context = androidx.compose.ui.platform.LocalContext.current
  val scope = rememberCoroutineScope()
  var lists by remember { mutableStateOf<List<RList>>(emptyList()) }
  var counts by remember { mutableStateOf<RCounts?>(null) }
  var signedIn by remember { mutableStateOf(true) }
  var picking by remember { mutableStateOf(false) }

  LaunchedEffect(Unit) {
    val result = withContext(Dispatchers.IO) {
      val engine = Engine.account(context)
      if (!engine.hasSession()) return@withContext Triple(false, emptyList<RList>(), null as RCounts?)
      val loadedLists = runCatching { engine.reminderLists() }.getOrDefault(emptyList())
      val loadedCounts = runCatching { engine.reminderCounts() }.getOrNull()
      Triple(true, loadedLists, loadedCounts)
    }
    signedIn = result.first
    lists = result.second
    counts = result.third
  }

  fun choose(kind: String, guid: String?) {
    if (picking) return
    picking = true
    scope.launch { onPick(kind, guid) }
  }

  Scaffold(
    topBar = {
      TopAppBar(
        title = { Text("Choose Reminders List") },
        navigationIcon = {
          IconButton(onClick = onCancel) {
            Icon(painterResource(R.drawable.ic_close), contentDescription = "Close")
          }
        },
      )
    },
  ) { inner ->
    LazyColumn(
      modifier = Modifier.fillMaxSize().padding(inner),
      contentPadding = PaddingValues(horizontal = 16.dp, vertical = 12.dp),
      verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
      item { SectionLabel("Smart Lists") }
      item {
        Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large) {
          Column {
            SmartPlace.entries.forEachIndexed { index, place ->
              val icon = when (place) {
                SmartPlace.ALL -> R.drawable.ic_inbox
                SmartPlace.TODAY -> R.drawable.ic_today
                SmartPlace.SCHEDULED -> R.drawable.ic_calendar
                SmartPlace.FLAGGED -> R.drawable.ic_flag
              }
              val smartCount = counts?.let {
                when (place) {
                  SmartPlace.ALL -> it.all
                  SmartPlace.TODAY -> it.today
                  SmartPlace.SCHEDULED -> it.scheduled
                  SmartPlace.FLAGGED -> it.flagged
                }
              }
              val colour = Color(place.accentArgb)
              Row(
                Modifier
                  .fillMaxWidth()
                  .clickable(enabled = !picking) { choose(place.key, null) }
                  .padding(horizontal = 16.dp, vertical = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
              ) {
                Box(
                  Modifier.size(32.dp).background(colour, CircleShape),
                  contentAlignment = Alignment.Center,
                ) {
                  Icon(painterResource(icon), contentDescription = null, tint = glyphOn(colour), modifier = Modifier.size(18.dp))
                }
                Spacer(Modifier.width(14.dp))
                Text(place.title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                if (smartCount != null) {
                  Text("$smartCount", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                } else {
                  Icon(painterResource(R.drawable.ic_chevron_right), contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
              }
              if (index < SmartPlace.entries.lastIndex) {
                HorizontalDivider(Modifier.padding(start = 62.dp), color = MaterialTheme.colorScheme.outlineVariant)
              }
            }
          }
        }
      }

      if (signedIn && lists.isNotEmpty()) {
        item { SectionLabel("My Lists") }
        item {
          Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large) {
            Column {
              lists.forEachIndexed { index, list ->
                val colour = parseHexColour(list.colour)
                Row(
                  Modifier
                    .fillMaxWidth()
                    .clickable(enabled = !picking) { choose("list", list.guid) }
                    .padding(horizontal = 16.dp, vertical = 14.dp),
                  verticalAlignment = Alignment.CenterVertically,
                ) {
                  Box(
                    Modifier.size(32.dp).background(colour, CircleShape),
                    contentAlignment = Alignment.Center,
                  ) {
                    Icon(painterResource(R.drawable.ic_list), contentDescription = null, tint = glyphOn(colour), modifier = Modifier.size(18.dp))
                  }
                  Spacer(Modifier.width(14.dp))
                  Text(list.title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                  Text("${list.open}", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (index < lists.lastIndex) {
                  HorizontalDivider(Modifier.padding(start = 62.dp), color = MaterialTheme.colorScheme.outlineVariant)
                }
              }
            }
          }
        }
      }
    }
  }
}

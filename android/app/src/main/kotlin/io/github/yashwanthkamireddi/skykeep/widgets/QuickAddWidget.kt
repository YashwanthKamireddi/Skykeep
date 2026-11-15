package io.github.yashwanthkamireddi.skykeep.widgets

import android.appwidget.AppWidgetManager
import android.content.Context
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.datastore.preferences.core.Preferences
import androidx.glance.ColorFilter
import androidx.glance.GlanceId
import androidx.glance.GlanceModifier
import androidx.glance.GlanceTheme
import androidx.glance.Image
import androidx.glance.ImageProvider
import androidx.glance.LocalContext
import androidx.glance.LocalSize
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.appWidgetBackground
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.provideContent
import androidx.glance.background
import androidx.glance.currentState
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.padding
import androidx.glance.layout.size
import androidx.glance.layout.width
import androidx.glance.state.GlanceStateDefinition
import androidx.glance.state.PreferencesGlanceStateDefinition
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextStyle
import io.github.yashwanthkamireddi.skykeep.R

/**
 * Minimalist Material 3 quick-capture widget: shows open reminder counts at a glance, opens All
 * reminders on tap, and launches [QuickReminderActivity] from a concentric circular `+` button.
 */
class QuickAddWidget : GlanceAppWidget() {
  override val stateDefinition: GlanceStateDefinition<*> = PreferencesGlanceStateDefinition
  override val sizeMode = SizeMode.Exact
  override val previewSizeMode = SizeMode.Responsive(setOf(DpSize(240.dp, 56.dp)))

  override suspend fun provideGlance(context: Context, id: GlanceId) {
    provideContent {
      val prefs = currentState<Preferences>()
      val tick by WidgetUpdates.remindersTick.collectAsState()
      val revision = prefs[WidgetPrefs.REVISION] ?: 0L
      val snapshot = remember(revision, tick) { WidgetData.loadReminders(context) }
      SkykeepWidgetTheme {
        QuickAddContent(snapshot)
      }
    }
  }

  override suspend fun providePreview(context: Context, widgetCategory: Int) {
    val sample = RemindersSnapshot(
      signedIn = true,
      viewKind = SmartPlace.ALL.key,
      listGuid = null,
      defaultTargetListGuid = null,
      title = "All",
      accentColor = Color(SmartPlace.ALL.accentArgb),
      counts = uniffi.quince_mobile.RCounts(today = 3u, scheduled = 5u, flagged = 1u, all = 8u, completed = 12u),
      lists = emptyList(),
      items = emptyList(),
      subItemGuids = emptySet(),
    )
    provideContent {
      SkykeepWidgetTheme {
        QuickAddContent(sample, isPreview = true)
      }
    }
  }
}

@Composable
private fun QuickAddContent(snapshot: RemindersSnapshot, isPreview: Boolean = false) {
  val context = LocalContext.current
  val size = LocalSize.current
  val showDetails = size.width >= 175.dp
  val allCount = snapshot.counts?.all?.toInt() ?: 0
  val todayCount = snapshot.counts?.today?.toInt() ?: 0

  val quickAddAction = if (isPreview) {
    actionStartActivity(WidgetIntents.openApp(context))
  } else if (snapshot.signedIn) {
    actionStartActivity(WidgetIntents.openQuickAdd(context, snapshot.defaultTargetListGuid, defaultToday = false))
  } else {
    actionStartActivity(WidgetIntents.openApp(context))
  }

  val openAllAction = actionStartActivity(
    WidgetIntents.openReminders(context, viewKind = SmartPlace.ALL.key, listTitle = SmartPlace.ALL.title),
  )

  Box(
    modifier = GlanceModifier
      .fillMaxSize()
      .appWidgetBackground()
      .background(GlanceTheme.colors.widgetBackground)
      .cornerRadius(R.dimen.widget_bg_radius)
      .padding(horizontal = 12.dp, vertical = 8.dp),
    contentAlignment = Alignment.Center,
  ) {
    Row(
      modifier = GlanceModifier.fillMaxSize(),
      verticalAlignment = Alignment.CenterVertically,
    ) {
      Row(
        modifier = GlanceModifier
          .defaultWeight()
          .let { if (isPreview) it else it.clickable(if (snapshot.signedIn) openAllAction else quickAddAction) },
        verticalAlignment = Alignment.CenterVertically,
      ) {
        Box(
          modifier = GlanceModifier
            .size(36.dp)
            .cornerRadius(18.dp)
            .background(GlanceTheme.colors.secondaryContainer),
          contentAlignment = Alignment.Center,
        ) {
          Image(
            provider = ImageProvider(R.drawable.ic_reminders),
            contentDescription = "Reminders",
            colorFilter = ColorFilter.tint(GlanceTheme.colors.onSecondaryContainer),
            modifier = GlanceModifier.size(18.dp),
          )
        }
        if (showDetails) {
          Spacer(GlanceModifier.width(12.dp))
          Column(GlanceModifier.defaultWeight()) {
            Text(
              text = if (snapshot.signedIn) "New reminder" else "Open Skykeep",
              maxLines = 1,
              style = TextStyle(
                color = GlanceTheme.colors.onSurface,
                fontSize = 15.sp,
                fontWeight = FontWeight.Medium,
              ),
            )
            if (snapshot.signedIn) {
              val summary = buildString {
                append("$allCount open")
                if (todayCount > 0) append(" · $todayCount today")
              }
              Text(
                text = summary,
                maxLines = 1,
                style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 12.sp),
              )
            } else {
              Text(
                text = "Sign in for reminders",
                maxLines = 1,
                style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 12.sp),
              )
            }
          }
        }
      }

      Spacer(GlanceModifier.width(8.dp))
      Box(
        modifier = GlanceModifier
          .size(38.dp)
          .cornerRadius(19.dp)
          .background(GlanceTheme.colors.primaryContainer)
          .clickable(quickAddAction),
        contentAlignment = Alignment.Center,
      ) {
        Image(
          provider = ImageProvider(R.drawable.ic_add),
          contentDescription = "Add reminder",
          colorFilter = ColorFilter.tint(GlanceTheme.colors.onPrimaryContainer),
          modifier = GlanceModifier.size(18.dp),
        )
      }
    }
  }
}

class QuickAddWidgetReceiver : GlanceAppWidgetReceiver() {
  override val glanceAppWidget: GlanceAppWidget = QuickAddWidget()

  override fun onUpdate(context: Context, appWidgetManager: AppWidgetManager, appWidgetIds: IntArray) {
    WidgetUpdates.bumpRemindersTick()
    super.onUpdate(context, appWidgetManager, appWidgetIds)
  }
}

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
import androidx.glance.action.Action
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.appWidgetBackground
import androidx.glance.appwidget.components.FilledButton
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.lazy.LazyColumn
import androidx.glance.appwidget.lazy.items
import androidx.glance.appwidget.provideContent
import androidx.glance.background
import androidx.glance.currentState
import androidx.glance.layout.Alignment
import androidx.glance.layout.Box
import androidx.glance.layout.Column
import androidx.glance.layout.Row
import androidx.glance.layout.Spacer
import androidx.glance.layout.fillMaxSize
import androidx.glance.layout.fillMaxWidth
import androidx.glance.layout.height
import androidx.glance.layout.padding
import androidx.glance.layout.size
import androidx.glance.layout.width
import androidx.glance.state.GlanceStateDefinition
import androidx.glance.state.PreferencesGlanceStateDefinition
import androidx.glance.text.FontWeight
import androidx.glance.text.Text
import androidx.glance.text.TextAlign
import androidx.glance.text.TextStyle
import androidx.glance.unit.ColorProvider
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.TextStyle as JavaTextStyle
import java.util.Locale

/**
 * Minimalist Material Design 3 Agenda widget for Android 15: shows today's and the next week's
 * events from Skykeep's synced iCloud calendars, with automatic smart sync and concentric layout.
 */
class AgendaWidget : GlanceAppWidget() {
  override val stateDefinition: GlanceStateDefinition<*> = PreferencesGlanceStateDefinition
  override val sizeMode = SizeMode.Exact
  override val previewSizeMode = SizeMode.Responsive(setOf(DpSize(260.dp, 220.dp)))

  override suspend fun provideGlance(context: Context, id: GlanceId) {
    provideContent {
      val prefs = currentState<Preferences>()
      val tick by WidgetUpdates.agendaTick.collectAsState()
      val revision = prefs[WidgetPrefs.REVISION] ?: 0L
      val snapshot = remember(revision, tick) { WidgetData.loadAgenda(context) }
      SkykeepWidgetTheme {
        AgendaWidgetContent(snapshot)
      }
    }
  }

  override suspend fun providePreview(context: Context, widgetCategory: Int) {
    val today = LocalDate.now()
    val baseMs = today.atTime(9, 30).atZone(ZoneId.systemDefault()).toInstant().toEpochMilli()
    val sample = AgendaSnapshot(
      signedIn = true,
      hasPermission = true,
      todayCount = 2,
      days = listOf(
        AgendaDayGroup(
          day = today,
          label = "Today",
          events = listOf(
            AgendaEventItem(1L, "Morning standup", baseMs, baseMs + 1_800_000L, false, "Conference Room", 0xFFE0313F.toInt(), "Work", today),
            AgendaEventItem(2L, "Lunch break", baseMs + 12_600_000L, baseMs + 16_200_000L, false, null, 0xFF2A82E0.toInt(), "Personal", today),
          ),
        ),
        AgendaDayGroup(
          day = today.plusDays(1),
          label = "Tomorrow",
          events = listOf(
            AgendaEventItem(3L, "Project milestone", 0L, 0L, true, null, 0xFF34C759.toInt(), "Work", today.plusDays(1)),
          ),
        ),
      ),
    )
    provideContent {
      SkykeepWidgetTheme {
        AgendaWidgetContent(sample, isPreview = true)
      }
    }
  }
}

private sealed interface AgendaRow {
  val key: String
  data class DayHeader(val label: String, override val key: String) : AgendaRow
  data class EventCard(val event: AgendaEventItem, override val key: String) : AgendaRow
}

@Composable
private fun AgendaWidgetContent(snapshot: AgendaSnapshot, isPreview: Boolean = false) {
  val context = LocalContext.current
  val today = LocalDate.now()
  val locale = Locale.getDefault()
  val dateTitle = "${today.dayOfWeek.getDisplayName(JavaTextStyle.SHORT, locale)}, ${today.month.getDisplayName(JavaTextStyle.SHORT, locale)} ${today.dayOfMonth}"

  Box(
    modifier = GlanceModifier
      .fillMaxSize()
      .appWidgetBackground()
      .background(GlanceTheme.colors.widgetBackground)
      .cornerRadius(R.dimen.widget_bg_radius)
      .padding(horizontal = 14.dp, vertical = 12.dp),
  ) {
    Column(GlanceModifier.fillMaxSize()) {
      Row(
        modifier = GlanceModifier
          .fillMaxWidth()
          .padding(start = 4.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        Row(
          modifier = GlanceModifier
            .defaultWeight()
            .let { if (isPreview) it else it.clickable(actionStartActivity(WidgetIntents.openCalendarApp(context))) }
            .padding(vertical = 4.dp),
          verticalAlignment = Alignment.CenterVertically,
        ) {
          Image(
            provider = ImageProvider(R.drawable.ic_calendar),
            contentDescription = null,
            colorFilter = ColorFilter.tint(ColorProvider(Color(Palette.App.Calendar.light))),
            modifier = GlanceModifier.size(18.dp),
          )
          Spacer(GlanceModifier.width(8.dp))
          Text(
            text = dateTitle,
            maxLines = 1,
            style = TextStyle(
              color = GlanceTheme.colors.onSurface,
              fontSize = 16.sp,
              fontWeight = FontWeight.Bold,
            ),
          )
          if (snapshot.signedIn && snapshot.hasPermission && snapshot.todayCount > 0) {
            Spacer(GlanceModifier.width(6.dp))
            Text(
              text = "${snapshot.todayCount}",
              maxLines = 1,
              style = TextStyle(
                color = GlanceTheme.colors.onSurfaceVariant,
                fontSize = 14.sp,
                fontWeight = FontWeight.Medium,
              ),
            )
          }
        }

        if (snapshot.signedIn && snapshot.hasPermission) {
          Spacer(GlanceModifier.width(8.dp))
          Box(
            modifier = GlanceModifier
              .size(36.dp)
              .cornerRadius(18.dp)
              .background(GlanceTheme.colors.primaryContainer)
              .clickable(
                actionStartActivity(
                  if (isPreview) WidgetIntents.openApp(context) else WidgetIntents.newCalendarEvent(),
                ),
              ),
            contentAlignment = Alignment.Center,
          ) {
            Image(
              provider = ImageProvider(R.drawable.ic_add),
              contentDescription = "New event",
              colorFilter = ColorFilter.tint(GlanceTheme.colors.onPrimaryContainer),
              modifier = GlanceModifier.size(18.dp),
            )
          }
        }
      }

      Box(
        modifier = GlanceModifier
          .fillMaxSize()
          .cornerRadius(R.dimen.widget_inner_radius)
          .background(GlanceTheme.colors.surface),
      ) {
        when {
          !snapshot.signedIn -> {
            AgendaEmptyState(
              icon = R.drawable.ic_calendar,
              title = "Sign in to Skykeep",
              subtitle = "Your synced iCloud events will appear here.",
              buttonText = "Open Skykeep",
              onAction = actionStartActivity(WidgetIntents.openApp(context)),
            )
          }
          !snapshot.hasPermission -> {
            AgendaEmptyState(
              icon = R.drawable.ic_calendar,
              title = "Calendar access needed",
              subtitle = "Allow Calendar in Skykeep so your events show here.",
              buttonText = "Open Skykeep",
              onAction = actionStartActivity(WidgetIntents.openApp(context)),
            )
          }
          snapshot.days.isEmpty() -> {
            AgendaEmptyState(
              icon = R.drawable.ic_check_circle,
              title = "No upcoming events",
              subtitle = "Nothing scheduled in the next 7 days.",
              buttonText = "New event",
              onAction = actionStartActivity(WidgetIntents.newCalendarEvent()),
            )
          }
          else -> {
            val rows = remember(snapshot.days) {
              buildList {
                for (group in snapshot.days) {
                  add(AgendaRow.DayHeader(group.label, "h-${group.day}"))
                  for (ev in group.events) {
                    add(AgendaRow.EventCard(ev, "e-${ev.eventId}-${ev.startMs}"))
                  }
                }
              }
            }
            LazyColumn(GlanceModifier.fillMaxSize().padding(horizontal = 12.dp, vertical = 6.dp)) {
              items(rows, itemId = { it.key.hashCode().toLong() }) { row ->
                when (row) {
                  is AgendaRow.DayHeader -> {
                    Text(
                      text = row.label,
                      style = TextStyle(
                        color = GlanceTheme.colors.primary,
                        fontSize = 11.sp,
                        fontWeight = FontWeight.Bold,
                      ),
                      modifier = GlanceModifier.padding(top = 6.dp, bottom = 2.dp),
                    )
                  }
                  is AgendaRow.EventCard -> {
                    AgendaEventRow(row.event, isPreview)
                  }
                }
              }
            }
          }
        }
      }
    }
  }
}

@Composable
private fun AgendaEventRow(event: AgendaEventItem, isPreview: Boolean) {
  val timeText = WidgetFormat.eventTimeRange(event.startMs, event.endMs, event.allDay)
  Row(
    modifier = GlanceModifier
      .fillMaxWidth()
      .let { m ->
        if (isPreview) m
        else m.clickable(actionStartActivity(WidgetIntents.openCalendarEvent(event.eventId, event.startMs, event.endMs)))
      }
      .padding(vertical = 6.dp),
    verticalAlignment = Alignment.CenterVertically,
  ) {
    Box(
      modifier = GlanceModifier
        .width(3.dp)
        .height(28.dp)
        .cornerRadius(2.dp)
        .background(ColorProvider(Color(event.calendarColor))),
    ) {}
    Spacer(GlanceModifier.width(10.dp))
    Column(GlanceModifier.defaultWeight()) {
      Text(
        text = event.title,
        maxLines = 1,
        style = TextStyle(
          color = GlanceTheme.colors.onSurface,
          fontSize = 14.sp,
          fontWeight = FontWeight.Medium,
        ),
      )
      Spacer(GlanceModifier.height(1.dp))
      val detail = buildString {
        append(timeText)
        event.location?.let { append(" · $it") }
      }
      Text(
        text = detail,
        maxLines = 1,
        style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 12.sp),
      )
    }
  }
}

@Composable
private fun AgendaEmptyState(
  icon: Int,
  title: String,
  subtitle: String,
  buttonText: String,
  onAction: Action,
) {
  Column(
    modifier = GlanceModifier.fillMaxSize().padding(16.dp),
    verticalAlignment = Alignment.CenterVertically,
    horizontalAlignment = Alignment.CenterHorizontally,
  ) {
    Image(
      provider = ImageProvider(icon),
      contentDescription = null,
      colorFilter = ColorFilter.tint(GlanceTheme.colors.primary),
      modifier = GlanceModifier.size(26.dp),
    )
    Spacer(GlanceModifier.height(8.dp))
    Text(
      text = title,
      style = TextStyle(color = GlanceTheme.colors.onSurface, fontSize = 15.sp, fontWeight = FontWeight.Medium),
    )
    Spacer(GlanceModifier.height(2.dp))
    Text(
      text = subtitle,
      style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 12.sp, textAlign = TextAlign.Center),
    )
    Spacer(GlanceModifier.height(12.dp))
    FilledButton(text = buttonText, onClick = onAction)
  }
}

class AgendaWidgetReceiver : GlanceAppWidgetReceiver() {
  override val glanceAppWidget: GlanceAppWidget = AgendaWidget()

  override fun onUpdate(context: Context, appWidgetManager: AppWidgetManager, appWidgetIds: IntArray) {
    WidgetUpdates.bumpAgendaTick()
    super.onUpdate(context, appWidgetManager, appWidgetIds)
  }
}

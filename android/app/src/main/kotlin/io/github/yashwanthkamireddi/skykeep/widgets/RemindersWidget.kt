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
import androidx.glance.action.Action
import androidx.glance.action.ActionParameters
import androidx.glance.action.actionParametersOf
import androidx.glance.action.clickable
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetReceiver
import androidx.glance.appwidget.SizeMode
import androidx.glance.appwidget.action.ActionCallback
import androidx.glance.appwidget.action.actionRunCallback
import androidx.glance.appwidget.action.actionStartActivity
import androidx.glance.appwidget.appWidgetBackground
import androidx.glance.appwidget.components.FilledButton
import androidx.glance.appwidget.cornerRadius
import androidx.glance.appwidget.lazy.LazyColumn
import androidx.glance.appwidget.lazy.items
import androidx.glance.appwidget.provideContent
import androidx.glance.appwidget.state.updateAppWidgetState
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
import uniffi.quince_mobile.RItem

private val KEY_REMINDER_GUID = ActionParameters.Key<String>("reminder_guid")
private val KEY_VIEW_KIND = ActionParameters.Key<String>("view_kind")

/**
 * Minimalist Material Design 3 Reminders widget for Android 15: defaults to All open reminders,
 * switches cleanly between All / Today / Scheduled / Flagged (or any configured iCloud list),
 * completes items in place, and syncs automatically without manual reload clutter.
 */
class RemindersWidget : GlanceAppWidget() {
  override val stateDefinition: GlanceStateDefinition<*> = PreferencesGlanceStateDefinition

  override val sizeMode = SizeMode.Exact
  override val previewSizeMode = SizeMode.Responsive(setOf(DpSize(260.dp, 220.dp)))

  override suspend fun provideGlance(context: Context, id: GlanceId) {
    provideContent {
      val prefs = currentState<Preferences>()
      val tick by WidgetUpdates.remindersTick.collectAsState()
      val viewKind = prefs[WidgetPrefs.VIEW_KIND]
      val listGuid = prefs[WidgetPrefs.LIST_GUID]
      val revision = prefs[WidgetPrefs.REVISION] ?: 0L
      val snapshot = remember(viewKind, listGuid, revision, tick) {
        WidgetData.loadReminders(context, prefs)
      }
      SkykeepWidgetTheme {
        RemindersWidgetContent(snapshot)
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
      accentColor = Color(Palette.Smart.ALL),
      counts = null,
      lists = emptyList(),
      items = listOf(
        RItem("preview-1", "list", "Pick up groceries", null, null, false, false, 0L, null, null, false),
        RItem("preview-2", "list", "Review project notes", "Check shared folder", null, false, true, 5L, null, null, false),
        RItem("preview-3", "list", "Call dentist", null, null, false, false, 0L, null, null, false),
      ),
      subItemGuids = emptySet(),
    )
    provideContent {
      SkykeepWidgetTheme {
        RemindersWidgetContent(sample, isPreview = true)
      }
    }
  }
}

@Composable
private fun RemindersWidgetContent(
  snapshot: RemindersSnapshot,
  isPreview: Boolean = false,
) {
  val context = LocalContext.current
  val size = LocalSize.current
  val showFilterStrip = snapshot.signedIn && size.height >= 185.dp
  val compactWidth = size.width < 230.dp
  val headerIcon = when (snapshot.viewKind) {
    SmartPlace.ALL.key -> R.drawable.ic_inbox
    SmartPlace.TODAY.key -> R.drawable.ic_today
    SmartPlace.SCHEDULED.key -> R.drawable.ic_calendar
    SmartPlace.FLAGGED.key -> R.drawable.ic_flag
    else -> R.drawable.ic_list
  }

  val openPlaceIntent = WidgetIntents.openReminders(
    context = context,
    viewKind = snapshot.viewKind,
    listGuid = snapshot.listGuid,
    listTitle = snapshot.title,
  )
  val quickAddIntent = WidgetIntents.openQuickAdd(
    context = context,
    listGuid = snapshot.defaultTargetListGuid,
    defaultToday = snapshot.viewKind == SmartPlace.TODAY.key,
  )

  Box(
    modifier = GlanceModifier
      .fillMaxSize()
      .appWidgetBackground()
      .background(GlanceTheme.colors.widgetBackground)
      .cornerRadius(R.dimen.widget_bg_radius)
      .padding(horizontal = 14.dp, vertical = 12.dp),
  ) {
    Column(GlanceModifier.fillMaxSize()) {
      // Minimalist header: view title + count on left, concentric circular + on right.
      Row(
        modifier = GlanceModifier
          .fillMaxWidth()
          .padding(start = 4.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        Row(
          modifier = GlanceModifier
            .defaultWeight()
            .let { if (isPreview) it else it.clickable(actionStartActivity(openPlaceIntent)) }
            .padding(vertical = 4.dp),
          verticalAlignment = Alignment.CenterVertically,
        ) {
          Image(
            provider = ImageProvider(headerIcon),
            contentDescription = null,
            colorFilter = ColorFilter.tint(ColorProvider(snapshot.accentColor)),
            modifier = GlanceModifier.size(18.dp),
          )
          Spacer(GlanceModifier.width(8.dp))
          Text(
            text = snapshot.title,
            maxLines = 1,
            style = TextStyle(
              color = GlanceTheme.colors.onSurface,
              fontSize = 16.sp,
              fontWeight = FontWeight.Bold,
            ),
          )
          if (snapshot.signedIn && snapshot.items.isNotEmpty()) {
            Spacer(GlanceModifier.width(6.dp))
            Text(
              text = "${snapshot.items.size + snapshot.more}",
              maxLines = 1,
              style = TextStyle(
                color = GlanceTheme.colors.onSurfaceVariant,
                fontSize = 14.sp,
                fontWeight = FontWeight.Medium,
              ),
            )
          }
        }

        if (snapshot.signedIn) {
          Spacer(GlanceModifier.width(8.dp))
          Box(
            modifier = GlanceModifier
              .size(36.dp)
              .cornerRadius(18.dp)
              .background(GlanceTheme.colors.primaryContainer)
              .clickable(
                if (isPreview) actionStartActivity(WidgetIntents.openApp(context))
                else actionStartActivity(quickAddIntent),
              ),
            contentAlignment = Alignment.Center,
          ) {
            Image(
              provider = ImageProvider(R.drawable.ic_add),
              contentDescription = "New reminder",
              colorFilter = ColorFilter.tint(GlanceTheme.colors.onPrimaryContainer),
              modifier = GlanceModifier.size(18.dp),
            )
          }
        }
      }

      if (showFilterStrip) {
        FilterChipStrip(snapshot.viewKind, compactWidth, isPreview)
        Spacer(GlanceModifier.height(8.dp))
      }

      // Unified inner Material 3 surface card keeps rows calm and clutter-free.
      Box(
        modifier = GlanceModifier
          .fillMaxSize()
          .cornerRadius(R.dimen.widget_inner_radius)
          .background(GlanceTheme.colors.surface),
      ) {
        when {
          !snapshot.signedIn -> {
            SignedOutBody(
              message = "Sign in to Skykeep to view your iCloud reminders.",
              onOpen = actionStartActivity(WidgetIntents.openApp(context)),
            )
          }
          snapshot.items.isEmpty() -> {
            EmptyRemindersBody(
              title = if (snapshot.viewKind == SmartPlace.TODAY.key) "Nothing due today" else "No reminders",
              subtitle = "Tap + to add one.",
              onAdd = actionStartActivity(quickAddIntent),
            )
          }
          else -> {
            LazyColumn(GlanceModifier.fillMaxSize().padding(vertical = 4.dp)) {
              items(snapshot.items, itemId = { it.guid.hashCode().toLong() }) { item ->
                val isSub = item.guid in snapshot.subItemGuids
                ReminderWidgetRow(
                  item = item,
                  accent = snapshot.accentColor,
                  indented = isSub,
                  isPreview = isPreview,
                  onRowClick = actionStartActivity(
                    WidgetIntents.openReminders(
                      context = context,
                      viewKind = snapshot.viewKind,
                      listGuid = snapshot.listGuid,
                      listTitle = snapshot.title,
                      reminderGuid = item.guid,
                    ),
                  ),
                )
              }
              if (snapshot.more > 0) {
                item(itemId = Long.MIN_VALUE) {
                  Text(
                    text = "${snapshot.more} more",
                    modifier = GlanceModifier
                      .fillMaxWidth()
                      .padding(horizontal = 16.dp, vertical = 10.dp)
                      .let { if (isPreview) it else it.clickable(actionStartActivity(openPlaceIntent)) },
                    style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 13.sp),
                  )
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
private fun FilterChipStrip(activeKind: String, compact: Boolean, isPreview: Boolean) {
  Row(GlanceModifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
    for ((i, place) in SmartPlace.entries.withIndex()) {
      if (i > 0) Spacer(GlanceModifier.width(4.dp))
      val selected = activeKind == place.key
      Box(
        modifier = GlanceModifier
          .defaultWeight()
          .cornerRadius(14.dp)
          .let { m ->
            if (selected) m.background(GlanceTheme.colors.secondaryContainer)
            else m
          }
          .let { m ->
            if (isPreview) m
            else m.clickable(actionRunCallback<SwitchSmartViewAction>(actionParametersOf(KEY_VIEW_KIND to place.key)))
          }
          .padding(horizontal = if (compact) 4.dp else 6.dp, vertical = 5.dp),
        contentAlignment = Alignment.Center,
      ) {
        Text(
          text = place.title,
          maxLines = 1,
          style = TextStyle(
            color = if (selected) GlanceTheme.colors.onSecondaryContainer else GlanceTheme.colors.onSurfaceVariant,
            fontSize = if (compact) 10.sp else 11.sp,
            fontWeight = if (selected) FontWeight.Bold else FontWeight.Medium,
            textAlign = TextAlign.Center,
          ),
        )
      }
    }
  }
}

@Composable
private fun ReminderWidgetRow(
  item: RItem,
  accent: Color,
  indented: Boolean,
  isPreview: Boolean,
  onRowClick: Action,
) {
  val due = WidgetFormat.reminderDue(item.dueAt, item.allDay, item.completedAt)
  val marks = WidgetFormat.priorityMarks(item.priority)
  Row(
    modifier = GlanceModifier
      .fillMaxWidth()
      .padding(start = if (indented) 22.dp else 6.dp, end = 12.dp, top = 4.dp, bottom = 4.dp),
    verticalAlignment = Alignment.CenterVertically,
  ) {
    Box(
      modifier = GlanceModifier
        .size(36.dp)
        .cornerRadius(18.dp)
        .let { m ->
          if (isPreview) m
          else m.clickable(actionRunCallback<ToggleReminderAction>(actionParametersOf(KEY_REMINDER_GUID to item.guid)))
        },
      contentAlignment = Alignment.Center,
    ) {
      Image(
        provider = ImageProvider(R.drawable.widget_checkbox_ring),
        contentDescription = "Complete ${item.title}",
        colorFilter = ColorFilter.tint(ColorProvider(accent)),
        modifier = GlanceModifier.size(18.dp),
      )
    }
    Spacer(GlanceModifier.width(4.dp))
    Column(
      modifier = GlanceModifier
        .defaultWeight()
        .let { if (isPreview) it else it.clickable(onRowClick) }
        .padding(vertical = 3.dp),
    ) {
      Text(
        text = "$marks${item.title.ifBlank { "New Reminder" }}",
        maxLines = 2,
        style = TextStyle(
          color = GlanceTheme.colors.onSurface,
          fontSize = 14.sp,
          fontWeight = FontWeight.Medium,
        ),
      )
      val subtitle = due?.text ?: item.notes?.lineSequence()?.firstOrNull { it.isNotBlank() }
      if (subtitle != null) {
        Spacer(GlanceModifier.height(1.dp))
        Text(
          text = if (due != null && item.repeats) "$subtitle · Repeats" else subtitle,
          maxLines = 1,
          style = TextStyle(
            color = if (due?.overdue == true) GlanceTheme.colors.error else GlanceTheme.colors.onSurfaceVariant,
            fontSize = 12.sp,
          ),
        )
      }
    }
    if (item.flagged) {
      Spacer(GlanceModifier.width(6.dp))
      Image(
        provider = ImageProvider(R.drawable.ic_flag),
        contentDescription = "Flagged",
        colorFilter = ColorFilter.tint(ColorProvider(Color(Palette.Smart.FLAGGED))),
        modifier = GlanceModifier.size(14.dp),
      )
    }
  }
}

@Composable
private fun EmptyRemindersBody(
  title: String,
  subtitle: String,
  onAdd: Action,
) {
  Column(
    modifier = GlanceModifier.fillMaxSize().padding(16.dp),
    verticalAlignment = Alignment.CenterVertically,
    horizontalAlignment = Alignment.CenterHorizontally,
  ) {
    Image(
      provider = ImageProvider(R.drawable.ic_check_circle),
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
    FilledButton(
      text = "New reminder",
      icon = ImageProvider(R.drawable.ic_add),
      onClick = onAdd,
    )
  }
}

@Composable
private fun SignedOutBody(message: String, onOpen: Action) {
  Column(
    modifier = GlanceModifier.fillMaxSize().padding(16.dp),
    verticalAlignment = Alignment.CenterVertically,
    horizontalAlignment = Alignment.CenterHorizontally,
  ) {
    Image(
      provider = ImageProvider(R.drawable.ic_reminders),
      contentDescription = null,
      colorFilter = ColorFilter.tint(GlanceTheme.colors.primary),
      modifier = GlanceModifier.size(26.dp),
    )
    Spacer(GlanceModifier.height(8.dp))
    Text(
      text = message,
      style = TextStyle(color = GlanceTheme.colors.onSurfaceVariant, fontSize = 13.sp, textAlign = TextAlign.Center),
    )
    Spacer(GlanceModifier.height(12.dp))
    FilledButton(text = "Open Skykeep", onClick = onOpen)
  }
}

class ToggleReminderAction : ActionCallback {
  override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
    val guid = parameters[KEY_REMINDER_GUID] ?: return
    if (WidgetData.toggleReminder(context, guid, currentlyDone = false)) {
      WidgetUpdates.reminders(context)
    }
  }
}

class SwitchSmartViewAction : ActionCallback {
  override suspend fun onAction(context: Context, glanceId: GlanceId, parameters: ActionParameters) {
    val kind = parameters[KEY_VIEW_KIND] ?: return
    updateAppWidgetState(context, glanceId) { prefs ->
      prefs[WidgetPrefs.VIEW_KIND] = kind
      prefs.remove(WidgetPrefs.LIST_GUID)
      prefs[WidgetPrefs.REVISION] = System.nanoTime()
    }
    WidgetUpdates.bumpRemindersTick()
    RemindersWidget().update(context, glanceId)
  }
}

class RemindersWidgetReceiver : GlanceAppWidgetReceiver() {
  override val glanceAppWidget: GlanceAppWidget = RemindersWidget()

  override fun onUpdate(context: Context, appWidgetManager: AppWidgetManager, appWidgetIds: IntArray) {
    WidgetUpdates.bumpRemindersTick()
    super.onUpdate(context, appWidgetManager, appWidgetIds)
  }
}

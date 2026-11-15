package io.github.yashwanthkamireddi.skykeep.widgets

import android.Manifest
import android.appwidget.AppWidgetManager
import android.content.ContentResolver
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.provider.CalendarContract
import androidx.compose.ui.graphics.Color
import androidx.core.content.ContextCompat
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.glance.appwidget.GlanceAppWidget
import androidx.glance.appwidget.GlanceAppWidgetManager
import androidx.glance.appwidget.state.updateAppWidgetState
import io.github.yashwanthkamireddi.skykeep.Engine
import io.github.yashwanthkamireddi.skykeep.sync.ACCOUNT_TYPE
import io.github.yashwanthkamireddi.skykeep.sync.REMINDERS_AUTHORITY
import io.github.yashwanthkamireddi.skykeep.sync.ReminderAlerts
import io.github.yashwanthkamireddi.skykeep.sync.currentAccount
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZoneOffset
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import uniffi.quince_mobile.RCounts
import uniffi.quince_mobile.RItem
import uniffi.quince_mobile.RList
import uniffi.quince_mobile.RView

/** Preferences keys stored per widget instance in Glance's DataStore. */
object WidgetPrefs {
  val VIEW_KIND = stringPreferencesKey("view_kind")
  val LIST_GUID = stringPreferencesKey("list_guid")
  val REVISION = longPreferencesKey("revision")
}

/** Snapshot rendered by [RemindersWidget] and [QuickAddWidget]. */
data class RemindersSnapshot(
  val signedIn: Boolean,
  val viewKind: String,
  val listGuid: String?,
  val defaultTargetListGuid: String?,
  val title: String,
  val accentColor: Color,
  val counts: RCounts?,
  val lists: List<RList>,
  val items: List<RItem>,
  val subItemGuids: Set<String>,
  /** Open reminders past [WidgetUpdates.MOST_ROWS], not drawn: the widget says how many. */
  val more: Int = 0,
)

/** Snapshot rendered by [AgendaWidget]. */
data class AgendaSnapshot(
  val signedIn: Boolean,
  val hasPermission: Boolean,
  val days: List<AgendaDayGroup>,
  val todayCount: Int,
)

object WidgetData {
  fun loadReminders(context: Context, prefs: Preferences? = null): RemindersSnapshot {
    val engine = Engine.account(context)
    if (!engine.hasSession()) {
      return RemindersSnapshot(
        signedIn = false,
        viewKind = SmartPlace.ALL.key,
        listGuid = null,
        defaultTargetListGuid = null,
        title = SmartPlace.ALL.title,
        accentColor = Color(SmartPlace.ALL.accentArgb),
        counts = null,
        lists = emptyList(),
        items = emptyList(),
        subItemGuids = emptySet(),
      )
    }

    val lists = runCatching { engine.reminderLists() }.getOrDefault(emptyList())
    val counts = runCatching { engine.reminderCounts() }.getOrNull()

    val requestedKind = prefs?.get(WidgetPrefs.VIEW_KIND) ?: SmartPlace.ALL.key
    val requestedGuid = prefs?.get(WidgetPrefs.LIST_GUID)
    val chosenList = if (requestedKind == "list" && requestedGuid != null) {
      lists.firstOrNull { it.guid == requestedGuid }
    } else {
      null
    }

    val effectiveKind: String
    val title: String
    val accent: Color
    val rView: RView
    if (chosenList != null) {
      effectiveKind = "list"
      title = chosenList.title
      accent = parseHexColour(chosenList.colour)
      rView = RView.List(chosenList.guid, completed = false)
    } else {
      val smart = SmartPlace.fromKey(requestedKind) ?: SmartPlace.ALL
      effectiveKind = smart.key
      title = smart.title
      accent = Color(smart.accentArgb)
      rView = when (smart) {
        SmartPlace.ALL -> RView.All
        SmartPlace.TODAY -> RView.Today
        SmartPlace.SCHEDULED -> RView.Scheduled
        SmartPlace.FLAGGED -> RView.Flagged
      }
    }

    val raw = runCatching { engine.reminders(rView) }.getOrDefault(emptyList())
      .filter { it.completedAt == null }
    // Group sub-tasks immediately under their parent, matching Reminders.kt.
    val listed = raw.mapTo(HashSet(raw.size)) { it.guid }
    val children = raw.filter { it.parentGuid != null && it.parentGuid in listed }.groupBy { it.parentGuid }
    val ordered = raw.filter { it.parentGuid == null || it.parentGuid !in listed }
      .flatMap { listOf(it) + children[it.guid].orEmpty() }
    val subGuids = ordered.filter { it.parentGuid != null && it.parentGuid in listed }.mapTo(HashSet()) { it.guid }

    // A widget is one RemoteViews, and Android refuses one too large ("Problem loading
    // widget"): a long list is cut, its rest counted.
    val shown = ordered.take(WidgetUpdates.MOST_ROWS)
    return RemindersSnapshot(
      signedIn = true,
      viewKind = effectiveKind,
      listGuid = chosenList?.guid,
      defaultTargetListGuid = chosenList?.guid ?: lists.firstOrNull()?.guid,
      title = title,
      accentColor = accent,
      counts = counts,
      lists = lists,
      items = shown,
      subItemGuids = subGuids,
      more = ordered.size - shown.size,
    )
  }

  fun toggleReminder(context: Context, guid: String, currentlyDone: Boolean): Boolean {
    val engine = Engine.account(context)
    if (!engine.hasSession()) return false
    val ok = runCatching { engine.reminderTick(guid, !currentlyDone) }.isSuccess
    if (!ok) return false
    runCatching { ReminderAlerts.schedule(context) }
    requestRemindersSync(context)
    return true
  }

  fun requestRemindersSync(context: Context) {
    currentAccount(context)?.let { account ->
      ContentResolver.requestSync(
        account,
        REMINDERS_AUTHORITY,
        Bundle().apply {
          putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
          putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
        },
      )
    }
  }

  fun requestCalendarSync(context: Context) {
    currentAccount(context)?.let { account ->
      ContentResolver.requestSync(
        account,
        CalendarContract.AUTHORITY,
        Bundle().apply {
          putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
          putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
        },
      )
    }
  }

  fun loadAgenda(context: Context): AgendaSnapshot {
    val engine = Engine.account(context)
    val account = currentAccount(context)
    if (!engine.hasSession() || account == null) {
      return AgendaSnapshot(signedIn = false, hasPermission = false, days = emptyList(), todayCount = 0)
    }
    if (ContextCompat.checkSelfPermission(context, Manifest.permission.READ_CALENDAR) != PackageManager.PERMISSION_GRANTED) {
      return AgendaSnapshot(signedIn = true, hasPermission = false, days = emptyList(), todayCount = 0)
    }

    val cr = context.contentResolver
    val visibleCalendars = HashMap<Long, Pair<String, Int>>()
    cr.query(
      CalendarContract.Calendars.CONTENT_URI,
      arrayOf(
        CalendarContract.Calendars._ID,
        CalendarContract.Calendars.CALENDAR_DISPLAY_NAME,
        CalendarContract.Calendars.CALENDAR_COLOR,
      ),
      "${CalendarContract.Calendars.ACCOUNT_TYPE}=? AND ${CalendarContract.Calendars.ACCOUNT_NAME}=? AND ${CalendarContract.Calendars.VISIBLE}=1",
      arrayOf(ACCOUNT_TYPE, account.name),
      null,
    )?.use { c ->
      while (c.moveToNext()) {
        visibleCalendars[c.getLong(0)] = (c.getString(1) ?: "") to c.getInt(2)
      }
    }

    if (visibleCalendars.isEmpty()) {
      return AgendaSnapshot(signedIn = true, hasPermission = true, days = emptyList(), todayCount = 0)
    }

    val zone = ZoneId.systemDefault()
    val today = LocalDate.now(zone)
    val lastDay = today.plusDays(6)
    val nowMs = System.currentTimeMillis()
    // Pad by a full day on both sides so UTC-midnight all-day instances never fall outside the query.
    val queryStartMs = today.minusDays(1).atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
    val queryEndMs = lastDay.plusDays(2).atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()

    val uri = CalendarContract.Instances.CONTENT_URI.buildUpon().apply {
      android.content.ContentUris.appendId(this, queryStartMs)
      android.content.ContentUris.appendId(this, queryEndMs)
    }.build()

    val events = ArrayList<AgendaEventItem>()
    cr.query(
      uri,
      arrayOf(
        CalendarContract.Instances.CALENDAR_ID,
        CalendarContract.Instances.EVENT_ID,
        CalendarContract.Instances.TITLE,
        CalendarContract.Instances.BEGIN,
        CalendarContract.Instances.END,
        CalendarContract.Instances.ALL_DAY,
        CalendarContract.Instances.EVENT_LOCATION,
        CalendarContract.Instances.DISPLAY_COLOR,
      ),
      null,
      null,
      "${CalendarContract.Instances.BEGIN} ASC",
    )?.use { c ->
      while (c.moveToNext()) {
        val calId = c.getLong(0)
        val calInfo = visibleCalendars[calId] ?: continue
        val eventId = c.getLong(1)
        val title = c.getString(2)?.takeIf { it.isNotBlank() } ?: "(No title)"
        val startMs = c.getLong(3)
        val endMs = c.getLong(4)
        val allDay = c.getInt(5) != 0
        val location = c.getString(6)?.takeIf { it.isNotBlank() }
        val color = if (c.isNull(7)) calInfo.second else c.getInt(7)

        // All-day instances are stored at UTC midnight; timed events end in the past if finished today.
        val day = if (allDay) {
          Instant.ofEpochMilli(startMs).atZone(ZoneOffset.UTC).toLocalDate()
        } else {
          Instant.ofEpochMilli(startMs).atZone(zone).toLocalDate()
        }
        if (day.isBefore(today) || day.isAfter(lastDay)) continue
        if (!allDay && day == today && endMs < nowMs) continue

        events += AgendaEventItem(
          eventId = eventId,
          title = title,
          startMs = startMs,
          endMs = endMs,
          allDay = allDay,
          location = location,
          calendarColor = color,
          calendarName = calInfo.first,
          day = day,
        )
      }
    }

    val days = groupAgendaByDay(events, today)
    val todayCount = days.firstOrNull { it.day == today }?.events?.size ?: 0
    return AgendaSnapshot(signedIn = true, hasPermission = true, days = days, todayCount = todayCount)
  }
}

/** Coordinates Glance widget updates, intelligent auto-sync, and Android 15 previews. */
object WidgetUpdates {

  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

  /** The most reminders one widget draws. */
  const val MOST_ROWS = 50

  private val _remindersTick = MutableStateFlow(0L)
  val remindersTick = _remindersTick.asStateFlow()

  private val _agendaTick = MutableStateFlow(0L)
  val agendaTick = _agendaTick.asStateFlow()

  fun bumpRemindersTick() {
    _remindersTick.value = System.nanoTime()
  }

  fun bumpAgendaTick() {
    _agendaTick.value = System.nanoTime()
  }

  private suspend fun <T : GlanceAppWidget> refreshAll(context: Context, widgetClass: Class<T>, factory: () -> T) {
    val app = context.applicationContext
    val manager = GlanceAppWidgetManager(app)
    val rev = System.nanoTime()
    val ids = manager.getGlanceIds(widgetClass)
    val widget = factory()
    for (id in ids) {
      updateAppWidgetState(app, id) { it[WidgetPrefs.REVISION] = rev }
      widget.update(app, id)
    }
  }

  suspend fun reminders(context: Context) {
    bumpRemindersTick()
    runCatching { refreshAll(context, RemindersWidget::class.java, ::RemindersWidget) }
    runCatching { refreshAll(context, QuickAddWidget::class.java, ::QuickAddWidget) }
  }

  suspend fun agenda(context: Context) {
    bumpAgendaTick()
    runCatching { refreshAll(context, AgendaWidget::class.java, ::AgendaWidget) }
  }

  suspend fun all(context: Context) {
    reminders(context)
    agenda(context)
  }

  fun scheduleReminders(context: Context) {
    val app = context.applicationContext
    scope.launch { reminders(app) }
  }

  fun scheduleAgenda(context: Context) {
    val app = context.applicationContext
    scope.launch { agenda(app) }
  }

  fun scheduleAll(context: Context) {
    val app = context.applicationContext
    scope.launch { all(app) }
  }

  /**
   * Publishes dynamic Material 3 previews for the Android 15+ widget picker when the system does
   * not hold one yet. Safe to call on every launch (checked via `generatedPreviewCategories`).
   */
  suspend fun publishPreviewsIfNeeded(context: Context) {
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.VANILLA_ICE_CREAM) return
    val app = context.applicationContext
    val appWidgetManager = app.getSystemService(AppWidgetManager::class.java) ?: return
    val glanceManager = GlanceAppWidgetManager(app)
    val receivers = listOf(
      RemindersWidgetReceiver::class,
      QuickAddWidgetReceiver::class,
      AgendaWidgetReceiver::class,
    )
    for (receiver in receivers) {
      val info = appWidgetManager.installedProviders.firstOrNull {
        it.provider.packageName == app.packageName && it.provider.className == receiver.java.name
      }
      if (info == null || info.generatedPreviewCategories == 0) {
        runCatching { glanceManager.setWidgetPreviews(receiver) }
      }
    }
  }
}

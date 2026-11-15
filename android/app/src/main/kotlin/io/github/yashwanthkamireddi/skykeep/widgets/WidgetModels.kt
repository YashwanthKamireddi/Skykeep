package io.github.yashwanthkamireddi.skykeep.widgets

import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.format.TextStyle
import java.util.Locale

/** Which Reminders view a widget shows when opened or configured. */
enum class SmartPlace(val key: String, val title: String, val accentArgb: Long) {
  ALL("all", "All", Palette.Smart.ALL),
  TODAY("today", "Today", Palette.Smart.TODAY),
  SCHEDULED("scheduled", "Scheduled", Palette.Smart.SCHEDULED),
  FLAGGED("flagged", "Flagged", Palette.Smart.FLAGGED);

  companion object {
    fun fromKey(key: String?): SmartPlace? = entries.firstOrNull { it.key == key }
  }
}

/**
 * Decides whether a widget should quietly ask for a background sync: only when not paused by
 * Advanced Data Protection and enough time has passed since the last sync or widget request.
 */
/** Formatted reminder due label and whether it is past due. */
data class DueDisplay(val text: String, val overdue: Boolean)

/** Pure formatting of a reminder's due timestamp, matching [Reminders.kt]. */
object WidgetFormat {
  fun priorityMarks(priority: Long): String = when (priority) {
    1L -> "!!! "
    5L -> "!! "
    9L -> "! "
    else -> ""
  }

  fun reminderDue(
    dueAtSeconds: Long?,
    allDay: Boolean,
    completedAt: Long?,
    now: Instant = Instant.now(),
    zone: ZoneId = ZoneId.systemDefault(),
    locale: Locale = Locale.getDefault(),
  ): DueDisplay? {
    val at = dueAtSeconds ?: return null
    val today = now.atZone(zone).toLocalDate()
    val moment = Instant.ofEpochSecond(at).atZone(zone)
    val date = moment.toLocalDate()
    val day = when (date) {
      today -> "Today"
      today.plusDays(1) -> "Tomorrow"
      today.minusDays(1) -> "Yesterday"
      else -> date.format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(locale))
    }
    val text = if (allDay) {
      day
    } else {
      val time = moment.toLocalTime()
        .format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale))
      "$day, $time"
    }
    val overdue = completedAt == null && if (allDay) {
      date.isBefore(today)
    } else {
      at < now.epochSecond
    }
    return DueDisplay(text, overdue)
  }

  fun eventTimeRange(
    startMs: Long,
    endMs: Long,
    allDay: Boolean,
    zone: ZoneId = ZoneId.systemDefault(),
    locale: Locale = Locale.getDefault(),
  ): String {
    if (allDay) return "All day"
    val fmt = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale)
    val start = Instant.ofEpochMilli(startMs).atZone(zone).toLocalTime().format(fmt)
    val end = Instant.ofEpochMilli(endMs).atZone(zone).toLocalTime().format(fmt)
    return if (endMs > startMs) "$start – $end" else start
  }

  fun agendaDayLabel(
    date: LocalDate,
    today: LocalDate,
    locale: Locale = Locale.getDefault(),
  ): String = when (date) {
    today -> "Today"
    today.plusDays(1) -> "Tomorrow"
    else -> {
      val dow = date.dayOfWeek.getDisplayName(TextStyle.SHORT, locale)
      val month = date.month.getDisplayName(TextStyle.SHORT, locale)
      "$dow, $month ${date.dayOfMonth}"
    }
  }
}

/** One calendar event instance shown by the Agenda widget. */
data class AgendaEventItem(
  val eventId: Long,
  val title: String,
  val startMs: Long,
  val endMs: Long,
  val allDay: Boolean,
  val location: String?,
  val calendarColor: Int,
  val calendarName: String,
  val day: LocalDate,
)

/** Events grouped under a single calendar day for the Agenda widget. */
data class AgendaDayGroup(
  val day: LocalDate,
  val label: String,
  val events: List<AgendaEventItem>,
)

/** Groups chronological events by day, putting all-day events first within each day. */
fun groupAgendaByDay(
  items: List<AgendaEventItem>,
  today: LocalDate,
  locale: Locale = Locale.getDefault(),
): List<AgendaDayGroup> = items
  .sortedWith(compareBy<AgendaEventItem> { it.day }.thenByDescending { it.allDay }.thenBy { it.startMs })
  .groupBy { it.day }
  .map { (day, dayEvents) ->
    AgendaDayGroup(
      day = day,
      label = WidgetFormat.agendaDayLabel(day, today, locale),
      events = dayEvents,
    )
  }

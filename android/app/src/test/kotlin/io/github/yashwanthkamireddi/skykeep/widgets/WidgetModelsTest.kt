package io.github.yashwanthkamireddi.skykeep.widgets

import java.time.Instant
import java.time.LocalDate
import java.time.ZoneOffset
import java.util.Locale
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class WidgetModelsTest {
  private val zone = ZoneOffset.UTC
  private val locale = Locale.US
  private val today = LocalDate.of(2026, 5, 14)
  private val now = today.atTime(14, 0).atZone(zone).toInstant()

  @Test
  fun smart_place_round_trips_keys() {
    for (place in SmartPlace.entries) {
      assertEquals(place, SmartPlace.fromKey(place.key))
    }
    assertNull(SmartPlace.fromKey(null))
    assertNull(SmartPlace.fromKey("unknown"))
  }

  @Test
  fun priority_marks_match_apple_reminders_levels() {
    assertEquals("!!! ", WidgetFormat.priorityMarks(1L))
    assertEquals("!! ", WidgetFormat.priorityMarks(5L))
    assertEquals("! ", WidgetFormat.priorityMarks(9L))
    assertEquals("", WidgetFormat.priorityMarks(0L))
  }

  @Test
  fun reminder_due_handles_relative_days_and_overdue() {
    assertNull(WidgetFormat.reminderDue(null, allDay = false, completedAt = null, now = now, zone = zone, locale = locale))

    // All-day today stays not overdue throughout the day.
    val todayAllDay = WidgetFormat.reminderDue(
      dueAtSeconds = today.atStartOfDay(zone).toEpochSecond(),
      allDay = true,
      completedAt = null,
      now = now,
      zone = zone,
      locale = locale,
    )!!
    assertEquals("Today", todayAllDay.text)
    assertFalse(todayAllDay.overdue)

    // Timed reminder earlier today is overdue while open, not overdue once completed.
    val earlierToday = today.atTime(9, 30).atZone(zone).toEpochSecond()
    val timedOverdue = WidgetFormat.reminderDue(earlierToday, allDay = false, completedAt = null, now = now, zone = zone, locale = locale)!!
    assertTrue(timedOverdue.text.startsWith("Today, "))
    assertTrue(timedOverdue.overdue)

    val timedDone = WidgetFormat.reminderDue(earlierToday, allDay = false, completedAt = now.epochSecond, now = now, zone = zone, locale = locale)!!
    assertFalse(timedDone.overdue)

    // Tomorrow & Yesterday labels.
    val tomorrow = WidgetFormat.reminderDue(
      dueAtSeconds = today.plusDays(1).atStartOfDay(zone).toEpochSecond(),
      allDay = true,
      completedAt = null,
      now = now,
      zone = zone,
      locale = locale,
    )!!
    assertEquals("Tomorrow", tomorrow.text)
    assertFalse(tomorrow.overdue)

    val yesterday = WidgetFormat.reminderDue(
      dueAtSeconds = today.minusDays(1).atStartOfDay(zone).toEpochSecond(),
      allDay = true,
      completedAt = null,
      now = now,
      zone = zone,
      locale = locale,
    )!!
    assertEquals("Yesterday", yesterday.text)
    assertTrue(yesterday.overdue)

    // Later date uses localized medium format.
    val nextWeek = WidgetFormat.reminderDue(
      dueAtSeconds = today.plusDays(5).atStartOfDay(zone).toEpochSecond(),
      allDay = true,
      completedAt = null,
      now = now,
      zone = zone,
      locale = locale,
    )!!
    assertEquals("May 19, 2026", nextWeek.text)
    assertFalse(nextWeek.overdue)
  }

  @Test
  fun event_time_range_formats_all_day_and_timed_events() {
    assertEquals("All day", WidgetFormat.eventTimeRange(0L, 0L, allDay = true, zone = zone, locale = locale))

    val start = today.atTime(9, 30).atZone(zone).toInstant().toEpochMilli()
    val end = today.atTime(10, 15).atZone(zone).toInstant().toEpochMilli()
    val range = WidgetFormat.eventTimeRange(start, end, allDay = false, zone = zone, locale = locale)
    assertTrue("Expected range with en-dash, got: $range", range.contains("–"))

    val instant = WidgetFormat.eventTimeRange(start, start, allDay = false, zone = zone, locale = locale)
    assertFalse("Zero-length event should not include range separator: $instant", instant.contains("–"))
  }

  @Test
  fun agenda_day_labels_and_grouping_put_all_day_first_chronologically() {
    assertEquals("Today", WidgetFormat.agendaDayLabel(today, today, locale))
    assertEquals("Tomorrow", WidgetFormat.agendaDayLabel(today.plusDays(1), today, locale))
    assertEquals("Sat, May 16", WidgetFormat.agendaDayLabel(today.plusDays(2), today, locale))

    val afternoon = today.atTime(15, 0).atZone(zone).toInstant().toEpochMilli()
    val morning = today.atTime(9, 0).atZone(zone).toInstant().toEpochMilli()
    val tomorrowStart = today.plusDays(1).atTime(11, 0).atZone(zone).toInstant().toEpochMilli()
    val allDayMs = Instant.from(today.atStartOfDay(zone)).toEpochMilli()

    val events = listOf(
      AgendaEventItem(1L, "Afternoon sync", afternoon, afternoon + 3_600_000L, false, null, 0, "Work", today),
      AgendaEventItem(2L, "Tomorrow review", tomorrowStart, tomorrowStart + 1_800_000L, false, "Room B", 0, "Work", today.plusDays(1)),
      AgendaEventItem(3L, "Company holiday", allDayMs, allDayMs, true, null, 0, "Personal", today),
      AgendaEventItem(4L, "Morning standup", morning, morning + 1_800_000L, false, null, 0, "Work", today),
    )

    val groups = groupAgendaByDay(events, today, locale)
    assertEquals(2, groups.size)
    assertEquals(today, groups[0].day)
    assertEquals("Today", groups[0].label)
    assertEquals(listOf("Company holiday", "Morning standup", "Afternoon sync"), groups[0].events.map { it.title })

    assertEquals(today.plusDays(1), groups[1].day)
    assertEquals("Tomorrow", groups[1].label)
    assertEquals(listOf("Tomorrow review"), groups[1].events.map { it.title })
  }
}

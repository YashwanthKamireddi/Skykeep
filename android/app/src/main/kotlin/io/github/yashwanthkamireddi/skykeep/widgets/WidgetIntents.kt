package io.github.yashwanthkamireddi.skykeep.widgets

import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.provider.CalendarContract
import io.github.yashwanthkamireddi.skykeep.MainActivity

/** Parsed navigation target when opening Skykeep from a Reminders widget. */
data class WidgetLaunchTarget(
  val id: Long,
  val viewKind: String,
  val listGuid: String?,
  val listTitle: String?,
  val reminderGuid: String?,
)

/**
 * Intent builders and extras shared between home screen widgets, [QuickReminderActivity], and
 * [MainActivity] so taps open the exact list, reminder, or calendar event.
 */
object WidgetIntents {
  const val EXTRA_TAB = "skykeep.widget.tab"
  const val EXTRA_VIEW = "skykeep.widget.view"
  const val EXTRA_LIST_GUID = "skykeep.widget.listGuid"
  const val EXTRA_LIST_TITLE = "skykeep.widget.listTitle"
  const val EXTRA_LIST_COLOUR = "skykeep.widget.listColour"
  const val EXTRA_REMINDER_GUID = "skykeep.widget.reminderGuid"
  const val EXTRA_DEFAULT_TODAY = "skykeep.widget.defaultToday"

  const val TAB_REMINDERS = "reminders"

  fun parseLaunchTarget(intent: Intent?, id: Long = System.nanoTime()): WidgetLaunchTarget? {
    if (intent?.getStringExtra(EXTRA_TAB) != TAB_REMINDERS) return null
    return WidgetLaunchTarget(
      id = id,
      viewKind = intent.getStringExtra(EXTRA_VIEW) ?: SmartPlace.ALL.key,
      listGuid = intent.getStringExtra(EXTRA_LIST_GUID),
      listTitle = intent.getStringExtra(EXTRA_LIST_TITLE),
      reminderGuid = intent.getStringExtra(EXTRA_REMINDER_GUID),
    )
  }

  fun openReminders(
    context: Context,
    viewKind: String = SmartPlace.ALL.key,
    listGuid: String? = null,
    listTitle: String? = null,
    listColour: Int? = null,
    reminderGuid: String? = null,
  ): Intent = Intent(context, MainActivity::class.java).apply {
    action = "io.github.yashwanthkamireddi.skykeep.OPEN_REMINDERS"
    flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
    putExtra(EXTRA_TAB, TAB_REMINDERS)
    putExtra(EXTRA_VIEW, viewKind)
    listGuid?.let { putExtra(EXTRA_LIST_GUID, it) }
    listTitle?.let { putExtra(EXTRA_LIST_TITLE, it) }
    listColour?.let { putExtra(EXTRA_LIST_COLOUR, it) }
    reminderGuid?.let { putExtra(EXTRA_REMINDER_GUID, it) }
  }

  fun openQuickAdd(
    context: Context,
    listGuid: String? = null,
    defaultToday: Boolean = false,
  ): Intent = Intent(context, QuickReminderActivity::class.java).apply {
    action = "io.github.yashwanthkamireddi.skykeep.QUICK_ADD_REMINDER"
    flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_MULTIPLE_TASK
    listGuid?.let { putExtra(EXTRA_LIST_GUID, it) }
    putExtra(EXTRA_DEFAULT_TODAY, defaultToday)
  }

  fun openApp(context: Context): Intent = Intent(context, MainActivity::class.java).apply {
    action = Intent.ACTION_MAIN
    flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP
  }

  fun openCalendarApp(context: Context): Intent = Intent(
    Intent.ACTION_VIEW,
    CalendarContract.CONTENT_URI.buildUpon().appendPath("time").build(),
  ).apply { flags = Intent.FLAG_ACTIVITY_NEW_TASK }

  fun openCalendarEvent(eventId: Long, startMs: Long, endMs: Long): Intent = Intent(
    Intent.ACTION_VIEW,
    ContentUris.withAppendedId(CalendarContract.Events.CONTENT_URI, eventId),
  ).apply {
    flags = Intent.FLAG_ACTIVITY_NEW_TASK
    putExtra(CalendarContract.EXTRA_EVENT_BEGIN_TIME, startMs)
    putExtra(CalendarContract.EXTRA_EVENT_END_TIME, endMs)
  }

  fun newCalendarEvent(): Intent = Intent(Intent.ACTION_INSERT).apply {
    data = CalendarContract.Events.CONTENT_URI
    flags = Intent.FLAG_ACTIVITY_NEW_TASK
  }
}

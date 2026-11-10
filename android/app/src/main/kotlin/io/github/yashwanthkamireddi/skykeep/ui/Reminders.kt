package io.github.yashwanthkamireddi.skykeep.ui

import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import android.Manifest
import android.content.ContentResolver
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TimePicker
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.sync.Outcome
import io.github.yashwanthkamireddi.skykeep.sync.REMINDERS_AUTHORITY
import io.github.yashwanthkamireddi.skykeep.sync.ReminderAlerts
import java.time.Instant
import java.time.LocalDate
import java.time.LocalTime
import java.time.ZoneId
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.RCounts
import uniffi.quince_mobile.REdit
import uniffi.quince_mobile.RItem
import uniffi.quince_mobile.RList
import uniffi.quince_mobile.RView

/** One of the overview's places: its title, its colour (Apple's), and how it is asked for. */
private data class Place(val title: String, val colour: Color, val view: RView, val listGuid: String? = null)

private fun colourOf(hex: String?, fallback: Color = Color(Palette.Smart.TODAY)): Color =
  hex?.removePrefix("#")?.takeIf { it.length == 6 }?.toLongOrNull(16)?.let { Color(0xFF000000 or it) } ?: fallback

/** Asks for a Reminders sync now: what was changed here goes, and Apple's is read. */
private fun syncReminders(context: android.content.Context) {
  phoneAccount(context)?.let {
    ContentResolver.requestSync(it, REMINDERS_AUTHORITY, Bundle().apply {
      putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
      putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
    })
  }
}

/**
 * Reminders, as the iPhone's app lays them out: Today, Scheduled, All and Flagged, with how many
 * are in each; Completed; then the person's lists. Everything is read from the phone's library,
 * so it opens at once and works offline; every change shows at once and is sent behind.
 */
@Composable
fun RemindersPage(engine: Account) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  var place by remember { mutableStateOf<Place?>(null) }
  var lists by remember { mutableStateOf<List<RList>>(emptyList()) }
  var counts by remember { mutableStateOf<RCounts?>(null) }
  var items by remember { mutableStateOf<List<RItem>>(emptyList()) }
  var showDone by remember { mutableStateOf(false) }
  var editing by remember { mutableStateOf<RItem?>(null) }
  var problem by remember { mutableStateOf<String?>(null) }
  var generation by remember { mutableIntStateOf(0) }

  fun reload() = scope.launch {
    val p = place
    runCatching {
      withContext(Dispatchers.IO) {
        Triple(
          engine.reminderLists(),
          engine.reminderCounts(),
          p?.let { engine.reminders(if (it.listGuid != null) RView.List(it.listGuid, showDone) else it.view) },
        )
      }
    }.onSuccess { (l, c, i) ->
      lists = l; counts = c; items = i ?: emptyList(); problem = null
    }.onFailure { problem = (it as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "Reminders couldn't be read." }
  }

  /** A change made here: written to the library, shown, then sent and the alerts set again. */
  fun change(write: () -> Unit) = scope.launch {
    val ok = runCatching { withContext(Dispatchers.IO) { write() } }
    ok.exceptionOrNull()?.let { problem = (it as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: it.message }
    reload()
    withContext(Dispatchers.IO) { ReminderAlerts.schedule(context) }
    syncReminders(context)
  }

  // Alerts need the permission on Android 13 and later: asked once, when Reminders first opens.
  val notify = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}
  LaunchedEffect(Unit) {
    if (Build.VERSION.SDK_INT >= 33 &&
      ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
    ) notify.launch(Manifest.permission.POST_NOTIFICATIONS)
    syncReminders(context)
  }
  // Read again whenever a sync finishes (Android runs them; the outcome is written when done).
  DisposableEffect(Unit) {
    val stop = Outcome.watch(context) { generation++ }
    onDispose { stop() }
  }
  LaunchedEffect(place, showDone, generation) { reload() }

  Column(Modifier.fillMaxSize()) {
    problem?.let {
      Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(20.dp, 12.dp))
    }
    val here = place
    if (here == null) {
      Overview(counts, lists) { place = it; showDone = false }
    } else {
      Row(Modifier.padding(start = 4.dp, end = 12.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        IconButton(onClick = { place = null }) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Back") }
        Text(here.title, style = MaterialTheme.typography.headlineSmall, color = here.colour, modifier = Modifier.weight(1f))
        if (here.listGuid != null) {
          TextButton(onClick = { showDone = !showDone }) { Text(if (showDone) "Hide Completed" else "Show Completed") }
        }
      }
      LazyColumn(Modifier.weight(1f), contentPadding = PaddingValues(horizontal = 20.dp, vertical = 4.dp)) {
        if (items.isEmpty()) {
          item {
            Text(
              if (here.listGuid != null) "No reminders" else "Nothing here",
              style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
              modifier = Modifier.padding(vertical = 32.dp),
            )
          }
        }
        items(items, key = { it.guid }) { r ->
          ReminderRow(r, here.colour, onTick = { change { engine.reminderTick(r.guid, r.completedAt == null) } }, onOpen = { editing = r })
          HorizontalDivider(Modifier.padding(start = 40.dp))
        }
      }
      // A new reminder goes in the list shown; in Reminders' own lists, into the first list.
      val into = here.listGuid ?: lists.firstOrNull()?.guid
      if (into != null) QuickAdd(here.colour) { title ->
        val today = if (here.view is RView.Today) LocalDate.now().atStartOfDay(ZoneId.systemDefault()).toEpochSecond() else null
        change { engine.reminderCreate(into, title, today, today != null) }
      }
    }
  }

  editing?.let { r ->
    EditSheet(r, lists, onDismiss = { editing = null }, onSave = { edit -> editing = null; change { engine.reminderEdit(r.guid, edit) } }, onDelete = {
      editing = null; change { engine.reminderDelete(r.guid) }
    })
  }
}

@Composable
private fun Overview(counts: RCounts?, lists: List<RList>, open: (Place) -> Unit) {
  LazyColumn(contentPadding = PaddingValues(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
    item { Text("Reminders", style = MaterialTheme.typography.headlineMedium) }
    item {
      val smart = listOf(
        Place("Today", Color(Palette.Smart.TODAY), RView.Today) to counts?.today,
        Place("Scheduled", Color(Palette.Smart.SCHEDULED), RView.Scheduled) to counts?.scheduled,
        Place("All", Color(Palette.Smart.ALL), RView.All) to counts?.all,
        Place("Flagged", Color(Palette.Smart.FLAGGED), RView.Flagged) to counts?.flagged,
      )
      Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
        for (pair in smart.chunked(2)) {
          Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            for ((p, n) in pair) SmartCard(p, n?.toInt(), Modifier.weight(1f)) { open(p) }
          }
        }
        val done = Place("Completed", Color(Palette.Smart.COMPLETED), RView.Completed)
        SmartCard(done, counts?.completed?.toInt(), Modifier.fillMaxWidth()) { open(done) }
      }
    }
    if (lists.isNotEmpty()) item { Text("My Lists", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 8.dp)) }
    items(lists, key = { it.guid }) { l ->
      val colour = colourOf(l.colour)
      Row(
        Modifier.fillMaxWidth().clickable { open(Place(l.title, colour, RView.List(l.guid, false), l.guid)) }.padding(vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        Box(Modifier.size(28.dp).background(colour, CircleShape))
        Spacer(Modifier.width(14.dp))
        Text(l.title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
        Text("${l.open}", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
      }
    }
  }
}

@Composable
private fun SmartCard(p: Place, n: Int?, modifier: Modifier, onClick: () -> Unit) {
  Card(modifier.clickable(onClick = onClick), colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainerHigh)) {
    Column(Modifier.padding(14.dp)) {
      Row(verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(28.dp).background(p.colour, CircleShape))
        Spacer(Modifier.weight(1f))
        Text(n?.toString() ?: "", style = MaterialTheme.typography.headlineSmall)
      }
      Spacer(Modifier.height(8.dp))
      Text(p.title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
  }
}

private val dayFormat: DateTimeFormatter = DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM)
private val timeFormat: DateTimeFormatter = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT)

/** A due date as Reminders says it: Today, Tomorrow, or the date; with the time unless all day. */
private fun dueText(r: RItem): String? {
  val at = r.dueAt ?: return null
  val zone = ZoneId.systemDefault()
  val moment = Instant.ofEpochSecond(at).atZone(zone)
  val day = when (moment.toLocalDate()) {
    LocalDate.now(zone) -> "Today"
    LocalDate.now(zone).plusDays(1) -> "Tomorrow"
    LocalDate.now(zone).minusDays(1) -> "Yesterday"
    else -> moment.toLocalDate().format(dayFormat)
  }
  return if (r.allDay) day else "$day, ${moment.toLocalTime().format(timeFormat)}"
}

@Composable
private fun ReminderRow(r: RItem, colour: Color, onTick: () -> Unit, onOpen: () -> Unit) {
  val done = r.completedAt != null
  val overdue = !done && (r.dueAt ?: Long.MAX_VALUE) < System.currentTimeMillis() / 1000 &&
    !(r.allDay && Instant.ofEpochSecond(r.dueAt ?: 0).atZone(ZoneId.systemDefault()).toLocalDate() == LocalDate.now())
  Row(Modifier.fillMaxWidth().clickable(onClick = onOpen).padding(vertical = 10.dp), verticalAlignment = Alignment.Top) {
    // Apple's circle: hollow while open, filled in the list's colour when done.
    Box(
      Modifier.padding(top = 2.dp).size(24.dp).clip(CircleShape)
        .border(2.dp, if (done) colour else MaterialTheme.colorScheme.outline, CircleShape)
        .background(if (done) colour else Color.Transparent, CircleShape)
        .clickable(onClick = onTick),
      contentAlignment = Alignment.Center,
    ) { if (done) Box(Modifier.size(8.dp).background(MaterialTheme.colorScheme.surface, CircleShape)) }
    Spacer(Modifier.width(16.dp))
    Column(Modifier.weight(1f)) {
      Text(
        r.title.ifBlank { "New Reminder" }, style = MaterialTheme.typography.bodyLarge,
        color = if (done) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
      )
      r.notes?.takeIf { it.isNotBlank() }?.let {
        Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
      }
      dueText(r)?.let {
        Text(
          if (r.repeats) "$it · Repeats" else it, style = MaterialTheme.typography.bodySmall,
          color = if (overdue) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
        )
      }
    }
    if (r.flagged) Icon(painterResource(R.drawable.ic_flag), contentDescription = "Flagged", tint = Color(Palette.Smart.FLAGGED), modifier = Modifier.size(18.dp))
  }
}

/** "New Reminder" at the foot of a list: typed, and Done on the keyboard adds it. */
@Composable
private fun QuickAdd(colour: Color, add: (String) -> Unit) {
  var text by remember { mutableStateOf("") }
  OutlinedTextField(
    value = text, onValueChange = { text = it },
    placeholder = { Text("New Reminder") },
    singleLine = true,
    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
    keyboardActions = KeyboardActions(onDone = { if (text.isNotBlank()) { add(text.trim()); text = "" } }),
    leadingIcon = { Box(Modifier.size(20.dp).border(2.dp, colour, CircleShape)) },
    shape = RoundedCornerShape(16.dp),
    modifier = Modifier.fillMaxWidth().imePadding().padding(16.dp),
  )
}

/**
 * A reminder's details, as the phone's own sheet has them: title, notes, date (with a time, or
 * all day), flag, priority, list; and Delete. Saved as it closes; only what changed is sent.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun EditSheet(r: RItem, lists: List<RList>, onDismiss: () -> Unit, onSave: (REdit) -> Unit, onDelete: () -> Unit) {
  val zone = ZoneId.systemDefault()
  var title by remember { mutableStateOf(r.title) }
  var notes by remember { mutableStateOf(r.notes ?: "") }
  var hasDate by remember { mutableStateOf(r.dueAt != null) }
  var allDay by remember { mutableStateOf(r.allDay || r.dueAt == null) }
  val start = r.dueAt?.let { Instant.ofEpochSecond(it).atZone(zone) }
  var day by remember { mutableStateOf(start?.toLocalDate() ?: LocalDate.now(zone)) }
  var time by remember { mutableStateOf(start?.toLocalTime()?.takeUnless { r.allDay } ?: LocalTime.of(9, 0)) }
  var flagged by remember { mutableStateOf(r.flagged) }
  var priority by remember { mutableStateOf(r.priority) }
  var listGuid by remember { mutableStateOf(r.listGuid) }
  var pickDay by remember { mutableStateOf(false) }
  var pickTime by remember { mutableStateOf(false) }

  fun close() {
    val due = if (!hasDate) null else if (allDay) day.atStartOfDay(zone).toEpochSecond() else day.atTime(time).atZone(zone).toEpochSecond()
    val dueChanged = due != r.dueAt || (hasDate && allDay != r.allDay)
    val edit = REdit(
      title = title.trim().takeIf { it != r.title && it.isNotEmpty() },
      notes = notes.takeIf { it.isNotBlank() && it != (r.notes ?: "") },
      clearNotes = notes.isBlank() && !r.notes.isNullOrBlank(),
      dueAt = if (dueChanged) due else null,
      allDay = if (dueChanged && due != null) allDay else null,
      clearDue = dueChanged && due == null,
      flagged = flagged.takeIf { it != r.flagged },
      priority = priority.takeIf { it != r.priority },
      listGuid = listGuid.takeIf { it != r.listGuid },
    )
    val changed = edit.title != null || edit.notes != null || edit.clearNotes || edit.dueAt != null || edit.clearDue ||
      edit.flagged != null || edit.priority != null || edit.listGuid != null
    if (changed) onSave(edit) else onDismiss()
  }

  ModalBottomSheet(onDismissRequest = ::close) {
    Column(Modifier.padding(horizontal = 24.dp).padding(bottom = 32.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
      OutlinedTextField(title, { title = it }, label = { Text("Title") }, modifier = Modifier.fillMaxWidth())
      OutlinedTextField(notes, { notes = it }, label = { Text("Notes") }, minLines = 2, modifier = Modifier.fillMaxWidth())
      Row(verticalAlignment = Alignment.CenterVertically) {
        Text("Date", style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        Switch(hasDate, { hasDate = it })
      }
      if (hasDate) {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
          TextButton(onClick = { pickDay = true }) { Text(day.format(dayFormat)) }
          if (!allDay) TextButton(onClick = { pickTime = true }) { Text(time.format(timeFormat)) }
          Spacer(Modifier.weight(1f))
          Text("All Day", style = MaterialTheme.typography.bodyMedium)
          Spacer(Modifier.width(8.dp))
          Switch(allDay, { allDay = it })
        }
      }
      Row(verticalAlignment = Alignment.CenterVertically) {
        Text("Flag", style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
        Switch(flagged, { flagged = it })
      }
      Text("Priority", style = MaterialTheme.typography.bodyLarge)
      val levels = listOf("None" to 0L, "Low" to 9L, "Medium" to 5L, "High" to 1L)
      SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        levels.forEachIndexed { i, (label, value) ->
          SegmentedButton(
            selected = priority == value, onClick = { priority = value },
            shape = SegmentedButtonDefaults.itemShape(i, levels.size),
          ) { Text(label) }
        }
      }
      if (lists.size > 1) {
        Text("List", style = MaterialTheme.typography.bodyLarge)
        Column {
          for (l in lists) {
            Row(
              Modifier.fillMaxWidth().clickable { listGuid = l.guid }.padding(vertical = 8.dp),
              verticalAlignment = Alignment.CenterVertically,
            ) {
              Box(Modifier.size(16.dp).background(colourOf(l.colour), CircleShape))
              Spacer(Modifier.width(12.dp))
              Text(l.title, modifier = Modifier.weight(1f))
              if (l.guid == listGuid) Text("✓", color = MaterialTheme.colorScheme.primary)
            }
          }
        }
      }
      TextButton(onClick = onDelete) { Text("Delete Reminder", color = MaterialTheme.colorScheme.error) }
    }
  }

  if (pickDay) {
    val state = rememberDatePickerState(initialSelectedDateMillis = day.atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli())
    DatePickerDialog(
      onDismissRequest = { pickDay = false },
      confirmButton = {
        TextButton(onClick = {
          state.selectedDateMillis?.let { day = Instant.ofEpochMilli(it).atZone(ZoneOffset.UTC).toLocalDate() }
          pickDay = false
        }) { Text("Done") }
      },
      dismissButton = { TextButton(onClick = { pickDay = false }) { Text("Cancel") } },
    ) { DatePicker(state) }
  }
  if (pickTime) {
    val state = rememberTimePickerState(time.hour, time.minute)
    DatePickerDialog(
      onDismissRequest = { pickTime = false },
      confirmButton = { TextButton(onClick = { time = LocalTime.of(state.hour, state.minute); pickTime = false }) { Text("Done") } },
      dismissButton = { TextButton(onClick = { pickTime = false }) { Text("Cancel") } },
    ) { Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) { TimePicker(state) } }
  }
}

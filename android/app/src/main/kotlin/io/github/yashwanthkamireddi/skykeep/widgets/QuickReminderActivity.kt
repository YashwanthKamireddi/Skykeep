package io.github.yashwanthkamireddi.skykeep.widgets

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DatePicker
import androidx.compose.material3.DatePickerDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TimePicker
import androidx.compose.material3.rememberDatePickerState
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import io.github.yashwanthkamireddi.skykeep.Engine
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.sync.ReminderAlerts
import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import io.github.yashwanthkamireddi.skykeep.ui.theme.SkykeepTheme
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
import uniffi.quince_mobile.REdit
import uniffi.quince_mobile.RList

/**
 * Translucent Material 3 bottom-sheet overlay launched from home screen widgets (`+` button):
 * lets the person add an iCloud reminder in one step without leaving their home screen.
 */
class QuickReminderActivity : ComponentActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val initialListGuid = intent?.getStringExtra(WidgetIntents.EXTRA_LIST_GUID)
    val defaultToday = intent?.getBooleanExtra(WidgetIntents.EXTRA_DEFAULT_TODAY, false) == true

    setContent {
      SkykeepTheme {
        QuickReminderSheet(
          initialListGuid = initialListGuid,
          defaultToday = defaultToday,
          onOpenApp = {
            startActivity(WidgetIntents.openApp(this))
            finish()
          },
          onDone = { finish() },
        )
      }
    }
  }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun QuickReminderSheet(
  initialListGuid: String?,
  defaultToday: Boolean,
  onOpenApp: () -> Unit,
  onDone: () -> Unit,
) {
  val context = androidx.compose.ui.platform.LocalContext.current
  val scope = rememberCoroutineScope()
  val zone = remember { ZoneId.systemDefault() }
  val today = remember { LocalDate.now(zone) }

  var loading by remember { mutableStateOf(true) }
  var signedIn by remember { mutableStateOf(false) }
  var lists by remember { mutableStateOf<List<RList>>(emptyList()) }
  var selectedList by remember { mutableStateOf<RList?>(null) }

  var title by remember { mutableStateOf("") }
  var showNotes by remember { mutableStateOf(false) }
  var notes by remember { mutableStateOf("") }
  var dueDate by remember { mutableStateOf(if (defaultToday) today else null) }
  var dueTime by remember { mutableStateOf<LocalTime?>(null) }
  var flagged by remember { mutableStateOf(false) }

  var showListMenu by remember { mutableStateOf(false) }
  var pickDate by remember { mutableStateOf(false) }
  var pickTime by remember { mutableStateOf(false) }
  var saving by remember { mutableStateOf(false) }
  var error by remember { mutableStateOf<String?>(null) }

  LaunchedEffect(Unit) {
    val result = withContext(Dispatchers.IO) {
      val engine = Engine.account(context)
      if (!engine.hasSession()) return@withContext false to emptyList<RList>()
      true to runCatching { engine.reminderLists() }.getOrDefault(emptyList())
    }
    signedIn = result.first
    lists = result.second
    selectedList = lists.firstOrNull { it.guid == initialListGuid } ?: lists.firstOrNull()
    loading = false
  }

  fun save() {
    val cleanTitle = title.trim()
    val target = selectedList ?: return
    if (cleanTitle.isEmpty() || saving) return
    saving = true
    error = null
    scope.launch {
      val result = withContext(Dispatchers.IO) {
        runCatching {
          val engine = Engine.account(context)
          val allDay = dueDate != null && dueTime == null
          val dueAt = dueDate?.let { d ->
            val t = dueTime
            if (t == null) d.atStartOfDay(zone).toEpochSecond()
            else d.atTime(t).atZone(zone).toEpochSecond()
          }
          val guid = engine.reminderCreate(target.guid, cleanTitle, dueAt, allDay)
          val cleanNotes = notes.trim()
          if (flagged || cleanNotes.isNotEmpty()) {
            engine.reminderEdit(
              guid,
              REdit(
                title = null,
                notes = cleanNotes.takeIf { it.isNotEmpty() },
                clearNotes = false,
                dueAt = null,
                allDay = null,
                clearDue = false,
                flagged = if (flagged) true else null,
                priority = null,
                listGuid = null,
              ),
            )
          }
          ReminderAlerts.schedule(context)
          WidgetData.requestRemindersSync(context)
          WidgetUpdates.reminders(context)
        }
      }
      saving = false
      result.onSuccess { onDone() }.onFailure {
        error = (it as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "Couldn’t save reminder."
      }
    }
  }

  Box(
    Modifier
      .fillMaxSize()
      .background(Color.Black.copy(alpha = 0.34f))
      .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onDone),
    contentAlignment = Alignment.BottomCenter,
  ) {
    Surface(
      shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp),
      color = MaterialTheme.colorScheme.surfaceContainer,
      modifier = Modifier
        .fillMaxWidth()
        .imePadding()
        .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {},
    ) {
      Column(
        Modifier.navigationBarsPadding().padding(horizontal = 24.dp).padding(top = 12.dp, bottom = 20.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
      ) {
        Box(
          Modifier
            .align(Alignment.CenterHorizontally)
            .size(width = 32.dp, height = 4.dp)
            .background(MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.35f), CircleShape),
        )
        when {
          loading -> {
            Row(
              Modifier.fillMaxWidth().padding(vertical = 16.dp),
              horizontalArrangement = Arrangement.spacedBy(12.dp),
              verticalAlignment = Alignment.CenterVertically,
            ) {
              CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
              Text("Loading lists…", style = MaterialTheme.typography.bodyLarge)
            }
          }
          !signedIn -> {
            Text("Sign in to Skykeep", style = MaterialTheme.typography.titleLarge)
            Text(
              "Open Skykeep and sign in to your iCloud account before adding reminders from a widget.",
              style = MaterialTheme.typography.bodyMedium,
              color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Row(Modifier.align(Alignment.End), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
              TextButton(onClick = onDone) { Text("Cancel") }
              Button(onClick = onOpenApp) { Text("Open Skykeep") }
            }
          }
          selectedList == null -> {
            Text("No reminder lists yet", style = MaterialTheme.typography.titleLarge)
            Text(
              "Open Skykeep once to finish syncing your iCloud reminder lists.",
              style = MaterialTheme.typography.bodyMedium,
              color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Row(Modifier.align(Alignment.End), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
              TextButton(onClick = onDone) { Text("Cancel") }
              Button(onClick = onOpenApp) { Text("Open Skykeep") }
            }
          }
          else -> {
            val focus = remember { FocusRequester() }
            LaunchedEffect(Unit) { focus.requestFocus() }

            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
              Text("New Reminder", style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
              IconButton(onClick = { showNotes = !showNotes }) {
                Icon(
                  painterResource(R.drawable.ic_notes),
                  contentDescription = if (showNotes) "Hide notes" else "Add notes",
                  tint = if (showNotes) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                )
              }
            }

            OutlinedTextField(
              value = title,
              onValueChange = { title = it },
              placeholder = { Text("What do you want to remember?") },
              singleLine = true,
              shape = RoundedCornerShape(16.dp),
              keyboardOptions = KeyboardOptions(
                capitalization = KeyboardCapitalization.Sentences,
                imeAction = ImeAction.Done,
              ),
              keyboardActions = KeyboardActions(onDone = { save() }),
              modifier = Modifier.fillMaxWidth().focusRequester(focus),
            )

            if (showNotes) {
              OutlinedTextField(
                value = notes,
                onValueChange = { notes = it },
                label = { Text("Notes") },
                minLines = 2,
                maxLines = 4,
                shape = RoundedCornerShape(16.dp),
                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                modifier = Modifier.fillMaxWidth(),
              )
            }

            Row(
              modifier = Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
              horizontalArrangement = Arrangement.spacedBy(8.dp),
              verticalAlignment = Alignment.CenterVertically,
            ) {
              Box {
                val chosen = selectedList!!
                val listColour = parseHexColour(chosen.colour)
                FilterChip(
                  selected = false,
                  onClick = { showListMenu = true },
                  label = { Text(chosen.title) },
                  leadingIcon = { Box(Modifier.size(10.dp).background(listColour, CircleShape)) },
                )
                DropdownMenu(expanded = showListMenu, onDismissRequest = { showListMenu = false }) {
                  for (l in lists) {
                    DropdownMenuItem(
                      leadingIcon = { Box(Modifier.size(12.dp).background(parseHexColour(l.colour), CircleShape)) },
                      text = { Text(l.title) },
                      onClick = {
                        selectedList = l
                        showListMenu = false
                      },
                    )
                  }
                }
              }

              FilterChip(
                selected = dueDate == today && dueTime == null,
                onClick = {
                  if (dueDate == today && dueTime == null) dueDate = null
                  else {
                    dueDate = today
                    dueTime = null
                  }
                },
                label = { Text("Today") },
                leadingIcon = { Icon(painterResource(R.drawable.ic_today), contentDescription = null, modifier = Modifier.size(16.dp)) },
              )

              FilterChip(
                selected = dueDate == today.plusDays(1) && dueTime == null,
                onClick = {
                  val tmrw = today.plusDays(1)
                  if (dueDate == tmrw && dueTime == null) dueDate = null
                  else {
                    dueDate = tmrw
                    dueTime = null
                  }
                },
                label = { Text("Tomorrow") },
              )

              val customDateSelected = dueDate != null && (dueTime != null || (dueDate != today && dueDate != today.plusDays(1)))
              val dateLabel = if (dueDate != null) {
                val dayFmt = DateTimeFormatter.ofLocalizedDate(FormatStyle.SHORT)
                val timeFmt = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT)
                val d = when (dueDate) {
                  today -> "Today"
                  today.plusDays(1) -> "Tomorrow"
                  else -> dueDate!!.format(dayFmt)
                }
                if (dueTime != null) "$d, ${dueTime!!.format(timeFmt)}" else d
              } else {
                "Date & time"
              }
              FilterChip(
                selected = customDateSelected,
                onClick = { pickDate = true },
                label = { Text(if (customDateSelected) dateLabel else "Date & time") },
                leadingIcon = { Icon(painterResource(R.drawable.ic_clock), contentDescription = null, modifier = Modifier.size(16.dp)) },
              )

              FilterChip(
                selected = flagged,
                onClick = { flagged = !flagged },
                label = { Text("Flag") },
                leadingIcon = {
                  Icon(
                    painterResource(R.drawable.ic_flag),
                    contentDescription = null,
                    tint = if (flagged) Color(Palette.Smart.FLAGGED) else MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(16.dp),
                  )
                },
              )
            }

            error?.let {
              Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodySmall)
            }

            Row(Modifier.align(Alignment.End), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
              TextButton(onClick = onDone, enabled = !saving) { Text("Cancel") }
              Button(onClick = ::save, enabled = title.isNotBlank() && !saving) {
                if (saving) {
                  CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onPrimary)
                  Spacer(Modifier.width(8.dp))
                }
                Text("Save")
              }
            }
          }
        }
      }
    }
  }

  if (pickDate) {
    val initialMillis = (dueDate ?: today).atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
    val state = rememberDatePickerState(initialSelectedDateMillis = initialMillis)
    DatePickerDialog(
      onDismissRequest = { pickDate = false },
      confirmButton = {
        TextButton(onClick = {
          state.selectedDateMillis?.let {
            dueDate = Instant.ofEpochMilli(it).atZone(ZoneOffset.UTC).toLocalDate()
          }
          pickDate = false
          pickTime = true
        }) { Text("Set Time") }
      },
      dismissButton = {
        Row {
          if (dueDate != null) {
            TextButton(onClick = { dueDate = null; dueTime = null; pickDate = false }) { Text("Clear") }
          }
          TextButton(onClick = {
            state.selectedDateMillis?.let {
              dueDate = Instant.ofEpochMilli(it).atZone(ZoneOffset.UTC).toLocalDate()
            }
            dueTime = null
            pickDate = false
          }) { Text("All Day") }
        }
      },
    ) { DatePicker(state) }
  }

  if (pickTime) {
    val initial = dueTime ?: LocalTime.of(9, 0)
    val state = rememberTimePickerState(initial.hour, initial.minute)
    DatePickerDialog(
      onDismissRequest = { pickTime = false },
      confirmButton = {
        TextButton(onClick = {
          dueTime = LocalTime.of(state.hour, state.minute)
          pickTime = false
        }) { Text("Done") }
      },
      dismissButton = {
        TextButton(onClick = { dueTime = null; pickTime = false }) { Text("No Time") }
      },
    ) {
      Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) {
        TimePicker(state)
      }
    }
  }
}

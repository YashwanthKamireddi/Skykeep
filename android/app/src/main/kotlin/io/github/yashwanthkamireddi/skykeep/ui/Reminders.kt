package io.github.yashwanthkamireddi.skykeep.ui

import androidx.compose.ui.graphics.toArgb
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import android.Manifest
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.AlertDialog
import androidx.compose.ui.graphics.luminance
import io.github.yashwanthkamireddi.skykeep.ui.theme.motion
import io.github.yashwanthkamireddi.skykeep.ui.theme.Motion
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.ui.draw.scale
import androidx.compose.foundation.selection.toggleable
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.material3.Surface
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
import uniffi.quince_mobile.AccountInterface
import uniffi.quince_mobile.RCounts
import uniffi.quince_mobile.REdit
import uniffi.quince_mobile.RItem
import uniffi.quince_mobile.RList
import uniffi.quince_mobile.RView

/** One of the overview's places: its title, its colour (Apple's), and how it is asked for. */
private data class Place(val title: String, val colour: Color, val view: RView, val listGuid: String? = null, val icon: Int = R.drawable.ic_list)

/** A place kept through a rotation: what it shows, as plain values. */
private val PlaceSaver = listSaver<Place?, Any>(
  save = { p ->
    if (p == null) emptyList() else {
      val v = p.view
      listOf(p.title, p.colour.toArgb(), p.icon, p.listGuid ?: "", when (v) {
        is RView.Today -> "today"; is RView.Scheduled -> "scheduled"; is RView.Flagged -> "flagged"
        is RView.All -> "all"; is RView.Completed -> "completed"; is RView.List -> "list"
      })
    }
  },
  restore = { s ->
    if (s.isEmpty()) null else {
      val guid = (s[3] as String).ifEmpty { null }
      val view = when (s[4]) {
        "today" -> RView.Today; "scheduled" -> RView.Scheduled; "flagged" -> RView.Flagged
        "completed" -> RView.Completed; "list" -> RView.List(guid ?: "", false); else -> RView.All
      }
      Place(s[0] as String, Color(s[1] as Int), view, guid, s[2] as Int)
    }
  },
)

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
fun RemindersPage(engine: AccountInterface) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  var place by rememberSaveable(stateSaver = PlaceSaver) { mutableStateOf<Place?>(null) }
  var lists by remember { mutableStateOf<List<RList>>(emptyList()) }
  var counts by remember { mutableStateOf<RCounts?>(null) }
  var items by remember { mutableStateOf<List<RItem>>(emptyList()) }
  var showDone by remember { mutableStateOf(false) }
  var editing by remember { mutableStateOf<RItem?>(null) }
  var listInfo by remember { mutableStateOf<RList?>(null) }
  var newList by remember { mutableStateOf(false) }
  var deleting by remember { mutableStateOf<RList?>(null) }
  /** Lists being deleted: hidden at once, while Apple takes the delete (it reads every reminder first). */
  var vanishing by remember { mutableStateOf<Set<String>>(emptySet()) }
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

  /** A list made, renamed, recoloured or deleted at Apple, then everything read again. */
  fun listChange(what: String, write: () -> Unit) = scope.launch {
    runCatching { withContext(Dispatchers.IO) { write() } }
      .onFailure { problem = (it as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "$what didn't reach iCloud." }
    reload()
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
  // Back from a list returns to the overview.
  androidx.activity.compose.BackHandler(enabled = place != null) { place = null }

  Column(Modifier.fillMaxSize()) {
    problem?.let {
      Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(20.dp, 12.dp))
    }
    val here = place
    if (here == null) {
      Overview(counts, lists.filter { it.guid !in vanishing }, onAddList = { newList = true }) { place = it; showDone = false }
    } else {
      // The desktop's list header: back, Show Completed; the list's name large in its colour,
      // and how many reminders it has.
      Row(Modifier.padding(start = 4.dp, end = 8.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        IconButton(onClick = { place = null }) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Back") }
        Spacer(Modifier.weight(1f))
        if (here.listGuid != null) {
          TextButton(onClick = { showDone = !showDone }) { Text(if (showDone) "Hide Completed" else "Show Completed") }
          val list = lists.firstOrNull { it.guid == here.listGuid }
          if (list != null) {
            var menu by remember { mutableStateOf(false) }
            Box {
              IconButton(onClick = { menu = true }) { Icon(painterResource(R.drawable.ic_more), contentDescription = "List options") }
              DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                DropdownMenuItem(text = { Text("Show List Info") }, onClick = { menu = false; listInfo = list })
                DropdownMenuItem(text = { Text("Delete List", color = MaterialTheme.colorScheme.error) }, onClick = { menu = false; deleting = list })
              }
            }
          }
        }
      }
      Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 8.dp)) {
        // The list's name in its colour where that reads (3:1, large text); a person's own list
        // colour that doesn't is never changed: the name goes ink, beside the list's mark.
        val bg = MaterialTheme.colorScheme.surface
        val readable = (maxOf(here.colour.luminance(), bg.luminance()) + 0.05f) / (minOf(here.colour.luminance(), bg.luminance()) + 0.05f) >= 3f
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
          if (!readable) Glyph(here.icon, here.colour, 32)
          Text(
            here.title, style = MaterialTheme.typography.headlineLarge,
            color = if (readable) here.colour else MaterialTheme.colorScheme.onSurface,
            maxLines = 2, overflow = TextOverflow.Ellipsis,
          )
        }
        val open = items.count { it.completedAt == null }
        Text(
          if (open == 1) "1 reminder" else "$open reminders",
          style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
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
        // Sub-tasks under their reminder, set in, as the desktop shows them.
        // By set, not by searching the list for each row: a list of thousands was laid out in
        // quadratic time on every redraw.
        val listed = items.mapTo(HashSet(items.size)) { it.guid }
        val children = items.filter { it.parentGuid != null && it.parentGuid in listed }.groupBy { it.parentGuid }
        val shown = items.filter { it.parentGuid == null || it.parentGuid !in listed }
          .flatMap { listOf(it) + children[it.guid].orEmpty() }
        items(shown, key = { it.guid }) { r ->
          val sub = r.parentGuid != null && r.parentGuid in listed
          ReminderRow(r, here.colour, sub, onTick = { change { engine.reminderTick(r.guid, r.completedAt == null) } }, onOpen = { editing = r })
          HorizontalDivider(Modifier.padding(start = if (sub) 76.dp else 40.dp), color = MaterialTheme.colorScheme.outlineVariant)
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

  if (newList) ListInfo(null, onDismiss = { newList = false }) { title, _ ->
    newList = false
    if (title.isNotEmpty()) listChange("Making $title") { engine.reminderListCreate(title) }
  }
  listInfo?.let { l ->
    ListInfo(l, onDismiss = { listInfo = null }) { title, colour ->
      listInfo = null
      listChange("Changing ${l.title}") {
        if (title.isNotEmpty() && title != l.title) engine.reminderListRename(l.guid, title)
        if (colour != null) engine.reminderListColour(l.guid, colour)
      }
      // The header shows the new name and colour at once; the lists are read again behind.
      place = place?.copy(title = title.ifEmpty { l.title }, colour = colour?.let { c -> LIST_COLOURS.first { it.first == c }.second } ?: place!!.colour)
    }
  }
  deleting?.let { l ->
    AlertDialog(
      onDismissRequest = { deleting = null },
      title = { Text("Delete “${l.title}”?") },
      text = { Text("The list and its reminders are deleted on all your devices.") },
      confirmButton = {
        TextButton(onClick = {
          deleting = null; place = null
          vanishing = vanishing + l.guid
          listChange("Deleting ${l.title}") { engine.reminderListDelete(l.guid) }.invokeOnCompletion { vanishing = vanishing - l.guid }
        }) { Text("Delete", color = MaterialTheme.colorScheme.error) }
      },
      dismissButton = { TextButton(onClick = { deleting = null }) { Text("Cancel") } },
    )
  }

  editing?.let { r ->
    EditSheet(r, lists, onDismiss = { editing = null }, onSave = { edit -> editing = null; change { engine.reminderEdit(r.guid, edit) } }, onDelete = {
      editing = null; change { engine.reminderDelete(r.guid) }
    })
  }
}

/**
 * Reminders' overview, the desktop's sidebar laid out for a phone: the four smart lists as tiles
 * (icon, count, name), Completed, then My Lists as one grouped card, each list with its icon in
 * its own colour and how many are open.
 */
@Composable
private fun Overview(counts: RCounts?, lists: List<RList>, onAddList: () -> Unit, open: (Place) -> Unit) {
  LazyColumn(contentPadding = PaddingValues(start = 16.dp, end = 16.dp, top = 20.dp, bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
    item { Text("Reminders", style = MaterialTheme.typography.headlineMedium, modifier = Modifier.padding(start = 4.dp, bottom = 4.dp)) }
    item {
      val smart = listOf(
        Place("Today", Color(Palette.Smart.TODAY), RView.Today, icon = R.drawable.ic_today) to counts?.today,
        Place("Scheduled", Color(Palette.Smart.SCHEDULED), RView.Scheduled, icon = R.drawable.ic_calendar) to counts?.scheduled,
        Place("All", Color(Palette.Smart.ALL), RView.All, icon = R.drawable.ic_inbox) to counts?.all,
        Place("Flagged", Color(Palette.Smart.FLAGGED), RView.Flagged, icon = R.drawable.ic_flag) to counts?.flagged,
      )
      Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        for (pair in smart.chunked(2)) {
          Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            for ((p, n) in pair) SmartTile(p, n?.toInt(), Modifier.weight(1f)) { open(p) }
          }
        }
        val done = Place("Completed", Color(Palette.Smart.COMPLETED), RView.Completed, icon = R.drawable.ic_check)
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
          SmartTile(done, counts?.completed?.toInt(), Modifier.weight(1f)) { open(done) }
          Spacer(Modifier.weight(1f))
        }
      }
    }
    if (lists.isNotEmpty()) {
      item {
        Text("My Lists", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(start = 4.dp, top = 12.dp))
      }
      item {
        Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large) {
          Column {
            lists.forEachIndexed { i, l ->
              val colour = colourOf(l.colour)
              Row(
                Modifier.fillMaxWidth().clickable { open(Place(l.title, colour, RView.List(l.guid, false), l.guid)) }
                  .padding(horizontal = 14.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
              ) {
                Glyph(R.drawable.ic_list, colour, 32)
                Spacer(Modifier.width(14.dp))
                Text(l.title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text("${l.open}", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
              }
              if (i < lists.lastIndex) HorizontalDivider(Modifier.padding(start = 60.dp), color = MaterialTheme.colorScheme.outlineVariant)
            }
          }
        }
      }
    }
    item {
      TextButton(onClick = onAddList) {
        Icon(painterResource(R.drawable.ic_add), contentDescription = null, modifier = Modifier.size(18.dp))
        Spacer(Modifier.width(8.dp))
        Text("Add List")
      }
    }
  }
}

/** A mark in a filled circle of [colour], its glyph white or ink, whichever reads on it. */
@Composable
private fun Glyph(icon: Int, colour: Color, size: Int) {
  Box(Modifier.size(size.dp).background(colour, CircleShape), contentAlignment = Alignment.Center) {
    Icon(painterResource(icon), contentDescription = null, tint = glyphOn(colour), modifier = Modifier.size((size * 0.56f).dp))
  }
}

/** One smart list as the desktop draws it: its icon, the count large, the name under it. */
@Composable
private fun SmartTile(p: Place, n: Int?, modifier: Modifier, onClick: () -> Unit) {
  Surface(
    onClick = onClick, color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large,
    modifier = modifier.semantics(mergeDescendants = true) {},
  ) {
    Column(Modifier.padding(start = 12.dp, end = 14.dp, top = 12.dp, bottom = 10.dp)) {
      Row(verticalAlignment = Alignment.CenterVertically) {
        Glyph(p.icon, p.colour, 32)
        Spacer(Modifier.weight(1f))
        Text(n?.toString() ?: "", style = MaterialTheme.typography.headlineMedium.copy(fontWeight = FontWeight.SemiBold))
      }
      Spacer(Modifier.height(6.dp))
      Text(p.title, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
  }
}

/** Apple's twelve list colours, in its picker's order, by the names Apple stores (the desktop's). */
private val LIST_COLOURS = listOf(
  "red" to Color(0xFFFF3B30), "orange" to Color(0xFFFF9500), "yellow" to Color(0xFFFFCC00),
  "green" to Color(0xFF34C759), "lightBlue" to Color(0xFF51AAF2), "blue" to Color(0xFF007AFF),
  "indigo" to Color(0xFF5856D6), "pink" to Color(0xFFEA426A), "purple" to Color(0xFFC077DB),
  "brown" to Color(0xFFA2845E), "gray" to Color(0xFF5B6770), "rose" to Color(0xFFD9A69F),
)

/**
 * A list's name and colour, as Show List Info has them; with no list, a new one's name (Apple
 * gives it its first colour; List Info changes it after). Answers the name and, if picked, the
 * colour's name.
 */
@Composable
private fun ListInfo(list: RList?, onDismiss: () -> Unit, onDone: (String, String?) -> Unit) {
  var title by remember { mutableStateOf(list?.title ?: "") }
  var colour by remember { mutableStateOf<String?>(null) }
  val current = list?.colour?.let { colourOf(it) }
  val keyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
  AlertDialog(
    onDismissRequest = { keyboard?.hide(); onDismiss() },
    title = { Text(if (list == null) "New List" else "List Info") },
    text = {
      Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        OutlinedTextField(title, { title = it }, singleLine = true, label = { Text("Name") }, modifier = Modifier.fillMaxWidth())
        if (list != null) {
          for (row in LIST_COLOURS.chunked(6)) {
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
              for ((name, c) in row) {
                val picked = colour == name || (colour == null && current == c)
                Box(
                  Modifier.size(32.dp).clip(CircleShape).background(c)
                    .border(if (picked) 3.dp else 0.dp, MaterialTheme.colorScheme.onSurface, CircleShape)
                    .clickable { colour = name }
                    .semantics { contentDescription = name + if (picked) ", chosen" else "" },
                )
              }
            }
          }
        }
      }
    },
    confirmButton = { TextButton(onClick = { keyboard?.hide(); onDone(title.trim(), colour) }, enabled = title.isNotBlank()) { Text("Done") } },
    dismissButton = { TextButton(onClick = { keyboard?.hide(); onDismiss() }) { Text("Cancel") } },
  )
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
private fun ReminderRow(r: RItem, colour: Color, sub: Boolean, onTick: () -> Unit, onOpen: () -> Unit) {
  val done = r.completedAt != null
  val overdue = !done && (r.dueAt ?: Long.MAX_VALUE) < System.currentTimeMillis() / 1000 &&
    !(r.allDay && Instant.ofEpochSecond(r.dueAt ?: 0).atZone(ZoneId.systemDefault()).toLocalDate() == LocalDate.now())
  Row(
    Modifier.fillMaxWidth().clickable(onClick = onOpen).padding(start = if (sub) 36.dp else 0.dp, top = 10.dp, bottom = 10.dp),
    verticalAlignment = Alignment.Top,
  ) {
    TickCircle(done, colour, onTick)
    Spacer(Modifier.width(14.dp))
    Column(Modifier.weight(1f)) {
      // Priority as the desktop marks it: ! low, !! medium, !!! high, in the list's colour.
      val marks = when (r.priority) { 1L -> "!!! "; 5L -> "!! "; 9L -> "! "; else -> "" }
      Text(
        buildAnnotatedString {
          if (marks.isNotEmpty()) withStyle(SpanStyle(color = colour, fontWeight = FontWeight.SemiBold)) { append(marks) }
          append(r.title.ifBlank { "New Reminder" })
        },
        style = MaterialTheme.typography.bodyLarge,
        color = if (done) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
      )
      r.notes?.takeIf { it.isNotBlank() }?.let {
        Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
      }
      dueText(r)?.let {
        Text(
          if (r.repeats) "$it · Repeats" else it, style = MaterialTheme.typography.bodyMedium,
          color = if (overdue) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
        )
      }
    }
    if (r.flagged) Icon(painterResource(R.drawable.ic_flag), contentDescription = "Flagged", tint = Color(Palette.Smart.FLAGGED), modifier = Modifier.padding(start = 8.dp, top = 2.dp).size(18.dp))
  }
}

/**
 * The tick circle: hollow while open; ticked, it fills in the list's colour with a check, on
 * the one expressive spring Skykeep has (instant when animations are off). 24 dp drawn, 48 dp
 * to touch; TalkBack hears a checkbox.
 */
@Composable
private fun TickCircle(done: Boolean, colour: Color, onTick: () -> Unit) {
  val fill by animateFloatAsState(if (done) 1f else 0f, motion(Motion.tick()), label = "tick")
  Box(
    Modifier.size(24.dp).toggleable(done, role = Role.Checkbox, onValueChange = { onTick() })
      .semantics { contentDescription = if (done) "Completed" else "Not completed" },
    contentAlignment = Alignment.Center,
  ) {
    Box(Modifier.size(24.dp).border(1.75.dp, if (done) colour else MaterialTheme.colorScheme.outline, CircleShape))
    Box(Modifier.size(24.dp).scale(fill).background(colour, CircleShape), contentAlignment = Alignment.Center) {
      if (fill > 0.5f) Icon(painterResource(R.drawable.ic_check), contentDescription = null, tint = glyphOn(colour), modifier = Modifier.size(14.dp))
    }
  }
}

/** "New Reminder" at the foot of a list: typed, and Done on the keyboard adds it. */
@Composable
private fun QuickAdd(colour: Color, add: (String) -> Unit) {
  var text by remember { mutableStateOf("") }
  Row(
    Modifier.fillMaxWidth().imePadding().background(MaterialTheme.colorScheme.surface).padding(horizontal = 20.dp, vertical = 12.dp),
    verticalAlignment = Alignment.CenterVertically,
  ) {
    Icon(painterResource(R.drawable.ic_add), contentDescription = null, tint = colour, modifier = Modifier.size(24.dp))
    Spacer(Modifier.width(14.dp))
    BasicTextField(
      value = text, onValueChange = { text = it },
      singleLine = true,
      textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
      cursorBrush = SolidColor(colour),
      keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done, capitalization = KeyboardCapitalization.Sentences),
      keyboardActions = KeyboardActions(onDone = { if (text.isNotBlank()) { add(text.trim()); text = "" } }),
      modifier = Modifier.weight(1f).semantics { contentDescription = "New Reminder" },
      decorationBox = { field ->
        if (text.isEmpty()) Text("New Reminder", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        field()
      },
    )
  }
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

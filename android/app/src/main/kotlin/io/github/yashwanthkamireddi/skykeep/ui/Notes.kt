package io.github.yashwanthkamireddi.skykeep.ui

import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import android.content.ContentResolver
import android.os.Bundle
import androidx.activity.compose.BackHandler
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.AlertDialog
import androidx.compose.ui.semantics.Role
import androidx.compose.foundation.selection.toggleable
import androidx.compose.ui.text.style.TextAlign
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
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.sync.NOTES_AUTHORITY
import io.github.yashwanthkamireddi.skykeep.sync.Outcome
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.AccountInterface
import uniffi.quince_mobile.NDoc
import uniffi.quince_mobile.NAttachment
import uniffi.quince_mobile.NFolder
import uniffi.quince_mobile.NItem
import uniffi.quince_mobile.NPart
import uniffi.quince_mobile.NSpan

/** Apple's id for Recently Deleted (quince-core's TRASH_FOLDER). */
private const val TRASH = "TrashFolder-CloudKit"

/** Notes' own yellow, as the iCloud Notes app has it. */
private val NotesYellow = Color(Palette.NOTES)

private fun syncNotes(context: android.content.Context) {
  phoneAccount(context)?.let {
    ContentResolver.requestSync(it, NOTES_AUTHORITY, Bundle().apply {
      putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
      putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
    })
  }
}

/** Where Notes is: the folders, one folder's notes (or all), or one note open. */
/** Where Notes is, kept through a rotation: kind, folder, folder title, note. */
private val NPlaceSaver = listSaver<NPlace, String>(
  save = { p ->
    when (p) {
      NPlace.Folders -> listOf("folders")
      is NPlace.Folder -> listOf("folder", p.guid ?: "", p.title)
      is NPlace.Open -> listOf("open", p.from.guid ?: "", p.from.title, p.guid ?: "")
    }
  },
  restore = { s ->
    when (s[0]) {
      "folder" -> NPlace.Folder(s[1].ifEmpty { null }, s[2])
      "open" -> NPlace.Open(s[3].ifEmpty { null }, NPlace.Folder(s[1].ifEmpty { null }, s[2]))
      else -> NPlace.Folders
    }
  },
)

private sealed interface NPlace {
  data object Folders : NPlace
  data class Folder(val guid: String?, val title: String) : NPlace
  /** A note open; `guid` null for a new one, made only once something is typed in it. */
  data class Open(val guid: String?, val from: Folder) : NPlace
}

/**
 * Notes, read from the phone's library: the folders with their counts, a folder's notes (pinned
 * first, then the latest), search, and a note open to read and edit. Every change shows at once
 * and is sent behind; a note's formatting is kept by the engine whatever is typed.
 */
@Composable
fun NotesPage(engine: AccountInterface) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  var place by rememberSaveable(stateSaver = NPlaceSaver) { mutableStateOf<NPlace>(NPlace.Folders) }
  var folders by remember { mutableStateOf<List<NFolder>>(emptyList()) }
  var notes by remember { mutableStateOf<List<NItem>>(emptyList()) }
  var query by remember { mutableStateOf("") }
  var problem by remember { mutableStateOf<String?>(null) }
  var generation by remember { mutableIntStateOf(0) }
  var newFolder by remember { mutableStateOf(false) }
  var renaming by remember { mutableStateOf<NFolder?>(null) }
  var dropping by remember { mutableStateOf<NFolder?>(null) }

  fun reason(t: Throwable) = (t as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "Notes couldn't be read."

  // The read in flight: a new one cancels it, so a search typed fast never shows an older
  // query's answer arriving after a newer one's.
  val reading = remember { arrayOfNulls<kotlinx.coroutines.Job>(1) }
  fun reload(): kotlinx.coroutines.Job {
    reading[0]?.cancel()
    return scope.launch {
      val p = place
      try {
        val (f, n) = withContext(Dispatchers.IO) {
          val f = engine.noteFolders()
          val n = when {
            query.isNotBlank() -> engine.searchNotes(query)
            p is NPlace.Folder -> engine.notes(p.guid)
            else -> emptyList()
          }
          f to n
        }
        folders = f; notes = n; problem = null
      } catch (e: kotlinx.coroutines.CancellationException) {
        throw e
      } catch (t: Throwable) {
        problem = reason(t)
      }
    }.also { reading[0] = it }
  }

  /** A folder made, renamed or deleted at Apple, then everything read again. */
  fun folderChange(what: String, write: () -> Unit) = scope.launch {
    runCatching { withContext(Dispatchers.IO) { write() } }.onFailure { problem = (it as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "$what didn't reach iCloud." }
    generation++
  }

  LaunchedEffect(Unit) { syncNotes(context) }
  DisposableEffect(Unit) {
    val stop = Outcome.watch(context) { generation++ }
    onDispose { stop() }
  }
  LaunchedEffect(place, query, generation) { reload() }
  // Back from a folder returns to the folders (the open note handles its own back: it saves).
  BackHandler(enabled = place is NPlace.Folder) { place = NPlace.Folders; query = "" }

  Column(Modifier.fillMaxSize()) {
    problem?.let { Text(it, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(20.dp, 12.dp)) }
    when (val p = place) {
      is NPlace.Open -> NoteScreen(engine, p.guid, p.from.guid, folders, onBack = { place = p.from; generation++ }, onProblem = { problem = it })
      else -> {
        val folder = p as? NPlace.Folder
        val trash = folder != null && folders.firstOrNull { it.guid == folder.guid }?.trash == true
        Row(Modifier.padding(start = 4.dp, end = 8.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
          if (folder != null) IconButton(onClick = { place = NPlace.Folders; query = "" }) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Folders") }
          Spacer(Modifier.weight(1f))
          if (folder != null && !trash) {
            // Nothing is made yet: a new note is made in iCloud when something is typed in it,
            // so one opened and left empty leaves nothing behind, as in Notes.
            IconButton(onClick = { place = NPlace.Open(null, folder) }) {
              Icon(painterResource(R.drawable.ic_edit), contentDescription = "New Note", tint = MaterialTheme.colorScheme.primary)
            }
          }
          // A folder of the person's own (not All iCloud, Notes' own, or Recently Deleted): Rename, Delete.
          val own = folders.firstOrNull { it.guid == folder?.guid && !it.trash }
          if (own != null) {
            var menu by remember { mutableStateOf(false) }
            Box {
              IconButton(onClick = { menu = true }) { Icon(painterResource(R.drawable.ic_more), contentDescription = "Folder options") }
              DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                DropdownMenuItem(text = { Text("Rename Folder") }, onClick = { menu = false; renaming = own })
                DropdownMenuItem(text = { Text("Delete Folder", color = MaterialTheme.colorScheme.error) }, onClick = { menu = false; dropping = own })
              }
            }
          }
        }
        Column(Modifier.padding(horizontal = 20.dp)) {
          Text(folder?.title ?: "Notes", style = MaterialTheme.typography.headlineLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
          if (folder != null && query.isBlank()) {
            Text(if (notes.size == 1) "1 note" else "${notes.size} notes", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
          }
        }
        SearchPill(query, { query = it }, Modifier.padding(horizontal = 16.dp, vertical = 12.dp), cursor = MaterialTheme.colorScheme.primary)
        if (folder == null && query.isBlank()) {
          FolderList(folders, onNewFolder = { newFolder = true }) { place = it }
        } else {
          NoteList(notes, grouped = query.isBlank()) { n -> place = NPlace.Open(n.guid, folder ?: NPlace.Folder(null, "All iCloud")) }
        }
      }
    }
  }
  if (newFolder) NameDialog("New Folder", "", onDismiss = { newFolder = false }) { name ->
    newFolder = false
    folderChange("Making $name") { engine.noteFolderCreate(name) }
  }
  renaming?.let { f ->
    NameDialog("Rename Folder", f.title, onDismiss = { renaming = null }) { name ->
      renaming = null
      if (name != f.title) {
        place = NPlace.Folder(f.guid, name)
        folderChange("Renaming ${f.title}") { engine.noteFolderRename(f.guid, name) }
      }
    }
  }
  dropping?.let { f ->
    AlertDialog(
      onDismissRequest = { dropping = null },
      title = { Text("Delete “${f.title}”?") },
      text = { Text("Its notes move to Recently Deleted on all your devices, where they stay for 30 days.") },
      confirmButton = {
        TextButton(onClick = {
          dropping = null; place = NPlace.Folders
          folderChange("Deleting ${f.title}") { engine.noteFolderDelete(f.guid) }
        }) { Text("Delete", color = MaterialTheme.colorScheme.error) }
      },
      dismissButton = { TextButton(onClick = { dropping = null }) { Text("Cancel") } },
    )
  }
}

/** The folders, as the desktop's sidebar lists them: All iCloud, each folder, Recently Deleted. */
@Composable
private fun FolderList(folders: List<NFolder>, onNewFolder: () -> Unit, open: (NPlace.Folder) -> Unit) {
  val all = folders.filter { !it.trash }.sumOf { it.count.toInt() }
  data class Row(val title: String, val count: Int, val icon: Int, val place: NPlace.Folder)
  val rows = buildList {
    add(Row("All iCloud", all, R.drawable.ic_folder, NPlace.Folder(null, "All iCloud")))
    // Apple's own "Notes" folder is the one with no folder: notes not filed anywhere.
    folders.filter { !it.trash }.forEach { add(Row(it.title, it.count.toInt(), R.drawable.ic_folder, NPlace.Folder(it.guid, it.title))) }
    folders.firstOrNull { it.trash }?.let { add(Row("Recently Deleted", it.count.toInt(), R.drawable.ic_trash, NPlace.Folder(it.guid, "Recently Deleted"))) }
  }
  LazyColumn(contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
    item {
      GroupCard(rows, inset = 54) { r ->
        androidx.compose.foundation.layout.Row(
          Modifier.fillMaxWidth().clickable { open(r.place) }.padding(horizontal = 16.dp, vertical = 14.dp),
          verticalAlignment = Alignment.CenterVertically,
        ) {
          Icon(painterResource(r.icon), contentDescription = null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(22.dp))
          Spacer(Modifier.width(16.dp))
          Text(r.title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
          Text("${r.count}", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
          Spacer(Modifier.width(6.dp))
          Icon(painterResource(R.drawable.ic_chevron_right), contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(18.dp))
        }
      }
    }
    item {
      TextButton(onClick = onNewFolder, modifier = Modifier.padding(top = 8.dp)) {
        Icon(painterResource(R.drawable.ic_folder_plus), contentDescription = null, modifier = Modifier.size(18.dp))
        Spacer(Modifier.width(8.dp))
        Text("New Folder")
      }
    }
  }
}

private val dayFormat: DateTimeFormatter = DateTimeFormatter.ofLocalizedDate(FormatStyle.SHORT)
private val timeFormat: DateTimeFormatter = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT)

/** When a note was changed, as Notes says it: the time today, Yesterday, the weekday this week, else the date. */
private fun changed(at: Long?): String {
  at ?: return ""
  val z = Instant.ofEpochSecond(at).atZone(ZoneId.systemDefault())
  val today = LocalDate.now()
  val d = z.toLocalDate()
  return when {
    d == today -> z.toLocalTime().format(timeFormat)
    d == today.minusDays(1) -> "Yesterday"
    d.isAfter(today.minusDays(7)) -> d.dayOfWeek.getDisplayName(java.time.format.TextStyle.FULL, java.util.Locale.getDefault())
    else -> d.format(dayFormat)
  }
}

/** Which section a note's last change puts it in, as Notes groups them. */
private fun bucket(n: NItem, today: LocalDate): String {
  if (n.pinned) return "Pinned"
  val d = n.modifiedAt?.let { Instant.ofEpochSecond(it).atZone(ZoneId.systemDefault()).toLocalDate() } ?: return "Earlier"
  return when {
    d == today -> "Today"
    d == today.minusDays(1) -> "Yesterday"
    d.isAfter(today.minusDays(7)) -> "Previous 7 Days"
    d.isAfter(today.minusDays(30)) -> "Previous 30 Days"
    d.year == today.year -> d.month.getDisplayName(java.time.format.TextStyle.FULL, java.util.Locale.getDefault())
    else -> d.year.toString()
  }
}

/**
 * Notes as the desktop lists them: grouped by when they changed (Pinned first), each section one
 * card, each note its title in bold over its time and first line. Search results are one list.
 */
@Composable
private fun NoteList(notes: List<NItem>, grouped: Boolean, open: (NItem) -> Unit) {
  val today = LocalDate.now()
  val sections = if (grouped) notes.groupBy { bucket(it, today) }.toList() else listOf("" to notes)
  LazyColumn(contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
    if (notes.isEmpty()) item {
      Text("No notes", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 4.dp, vertical = 24.dp))
    }
    for ((name, list) in sections) {
      if (name.isNotEmpty()) item(key = "h-$name") { SectionLabel(name) }
      groupItems(list, key = { "n-${it.guid}" }, inset = 16) { n ->
          Column(Modifier.fillMaxWidth().clickable { open(n) }.padding(horizontal = 16.dp, vertical = 12.dp)) {
            Text(n.title.ifBlank { "New Note" }, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Row {
              Text(changed(n.modifiedAt), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurface)
              Spacer(Modifier.width(8.dp))
              Text(
                if (n.locked) "Locked" else n.snippet?.lineSequence()?.firstOrNull { it.isNotBlank() } ?: "No additional text",
                style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis,
              )
            }
          }
      }
    }
  }
}

/** The colours of Notes' coloured text: 1 purple, 2 pink, 3 orange, 4 mint, 5 blue. */
private fun emphasisColour(e: UInt?): Color? = when (e?.toInt()) {
  1 -> Color(0xFFAF52DE); 2 -> Color(0xFFFF2D55); 3 -> Color(0xFFFF9500); 4 -> Color(0xFF00C7BE); 5 -> Color(0xFF007AFF)
  else -> null
}

/**
 * A note open. Read: drawn with its formatting, checklist items tickable. Edit (tap the text):
 * plain text, saved as typed; the engine carries every heading, list, link and attachment over
 * to the new text, so nothing the phone doesn't draw is lost. Locked notes are not opened.
 */
@Composable
private fun NoteScreen(engine: AccountInterface, opened: String?, folderGuid: String?, folders: List<NFolder>, onBack: () -> Unit, onProblem: (String) -> Unit) {
  var moving by remember { mutableStateOf(false) }
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  var guid by remember { mutableStateOf(opened) }
  var doc by remember { mutableStateOf<NDoc?>(null) }
  var editing by remember { mutableStateOf(opened == null) }
  var text by remember { mutableStateOf("") }
  var saved by remember { mutableStateOf("") }

  fun reason(t: Throwable) = (t as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "This note couldn't be saved."
  fun load() = scope.launch {
    val g = guid ?: return@launch
    runCatching { withContext(Dispatchers.IO) { engine.note(g) } }
      .onSuccess { doc = it; if (!editing) { text = it.text; saved = it.text } }
      .onFailure { onProblem(reason(it)) }
  }
  fun save(then: () -> Unit = {}) = scope.launch {
    if (text != saved) {
      val g = guid
      runCatching {
        withContext(Dispatchers.IO) {
          // A new note is made with its first text; one left empty is never made.
          if (g == null) { if (text.isNotBlank()) guid = engine.noteCreate(folderGuid, text) } else engine.noteSave(g, text)
        }
      }.onSuccess { saved = text; syncNotes(context) }.onFailure { onProblem(reason(it)) }
    }
    then()
  }

  LaunchedEffect(guid) { load() }
  // Typing is saved a moment after it stops, as Notes saves: never lost, never a save per key.
  LaunchedEffect(text, editing) {
    if (editing && text != saved) { delay(800); save() }
  }
  BackHandler { save { onBack() } }

  // A new note has no document yet: it is an empty one being typed.
  val d = doc ?: NDoc(guid ?: "", folderGuid, "", text, emptyList(), pinned = false, locked = false, editable = true, modifiedAt = null).takeIf { guid == null } ?: return
  Column(Modifier.fillMaxSize()) {
    // The desktop's note toolbar as phone actions: back; pin, delete (recover in Recently
    // Deleted); Done while typing. Each a mark in Notes' colour, named for TalkBack.
    val accent = MaterialTheme.colorScheme.primary
    Row(Modifier.padding(start = 4.dp, end = 4.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
      IconButton(onClick = { save { onBack() } }) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Back", tint = accent) }
      Spacer(Modifier.weight(1f))
      val g = guid
      // In Recently Deleted a note is recovered or deleted for good, as in Notes; not pinned or edited.
      val trashed = d.folderGuid == TRASH
      if (!d.locked && g != null) {
        if (trashed) {
          TextButton(onClick = { scope.launch { withContext(Dispatchers.IO) { runCatching { engine.noteMove(g, null) } }; syncNotes(context); onBack() } }) {
            Text("Recover")
          }
        } else {
          IconButton(onClick = { scope.launch { withContext(Dispatchers.IO) { runCatching { engine.notePin(g, !d.pinned) } }; syncNotes(context); load() } }) {
            Icon(painterResource(R.drawable.ic_pin), contentDescription = if (d.pinned) "Unpin" else "Pin", tint = if (d.pinned) accent else MaterialTheme.colorScheme.onSurfaceVariant)
          }
          IconButton(onClick = { moving = true }) {
            Icon(painterResource(R.drawable.ic_folder), contentDescription = "Move to Folder", tint = MaterialTheme.colorScheme.onSurfaceVariant)
          }
        }
        if (trashed) {
          TextButton(onClick = { scope.launch { withContext(Dispatchers.IO) { runCatching { engine.noteDelete(g) } }; syncNotes(context); onBack() } }) {
            Text("Delete Now", color = MaterialTheme.colorScheme.error)
          }
        } else {
          IconButton(onClick = { scope.launch { withContext(Dispatchers.IO) { runCatching { engine.noteDelete(g) } }; syncNotes(context); onBack() } }) {
            Icon(painterResource(R.drawable.ic_trash), contentDescription = "Delete", tint = MaterialTheme.colorScheme.onSurfaceVariant)
          }
        }
      }
      if (!d.locked && editing) TextButton(onClick = { save { editing = false; load() } }) { Text("Done") }
    }
    // When it last changed, small and centred above the note, as the desktop shows it.
    d.modifiedAt?.let { at ->
      Text(
        Instant.ofEpochSecond(at).atZone(ZoneId.systemDefault()).format(DateTimeFormatter.ofLocalizedDateTime(FormatStyle.LONG, FormatStyle.SHORT)),
        style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.fillMaxWidth().padding(bottom = 4.dp), textAlign = TextAlign.Center,
      )
    }
    when {
      d.locked -> Text(
        "This note is locked. Open it on an Apple device with its password.",
        style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(24.dp),
      )
      editing -> {
        // The keyboard comes up with the editor, as Notes opens a new or tapped note ready to type.
        val focus = remember { FocusRequester() }
        LaunchedEffect(Unit) { focus.requestFocus() }
        BasicTextField(
          text, { text = it },
          // An attachment shows as 📎, not the font's dotted "OBJ" box (discussion #199).
          visualTransformation = AttachmentMarks,
          textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
          cursorBrush = SolidColor(NotesYellow),
          modifier = Modifier.fillMaxSize().imePadding().padding(horizontal = 24.dp, vertical = 8.dp).focusRequester(focus),
        )
      }
      else -> RichNote(
        d,
        engine,
        Modifier.fillMaxSize().clickable(enabled = d.editable && d.folderGuid != TRASH) { text = d.text; saved = d.text; editing = true },
      ) { at, checked ->
        val g = guid ?: return@RichNote
        scope.launch {
          runCatching { withContext(Dispatchers.IO) { engine.noteCheck(g, at, checked) } }.onFailure { onProblem(reason(it)) }
          syncNotes(context); load()
        }
      }
    }
  }
  if (moving) {
    val g = guid
    AlertDialog(
      onDismissRequest = { moving = false },
      title = { Text("Move to Folder") },
      text = {
        Column {
          // Notes' own folder (no folder) first, then the person's; Recently Deleted is not a place to move to.
          val choices = listOf<Pair<String?, String>>(null to "Notes") + folders.filter { !it.trash }.map { it.guid to it.title }
          for ((to, title) in choices) {
            Row(
              Modifier.fillMaxWidth().clickable {
                moving = false
                if (g != null && to != d.folderGuid) scope.launch {
                  runCatching { withContext(Dispatchers.IO) { engine.noteMove(g, to) } }.onFailure { onProblem(reason(it)) }
                  syncNotes(context); load()
                }
              }.padding(vertical = 12.dp),
              verticalAlignment = Alignment.CenterVertically,
            ) {
              Icon(painterResource(R.drawable.ic_folder), contentDescription = null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(20.dp))
              Spacer(Modifier.width(14.dp))
              Text(title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
              if (to == d.folderGuid) Icon(painterResource(R.drawable.ic_check), contentDescription = "Here now", tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(18.dp))
            }
          }
        }
      },
      confirmButton = { TextButton(onClick = { moving = false }) { Text("Cancel") } },
    )
  }
}

/** One line of a note, laid out once: where it starts, its text, its paragraph and the spans on it. */
private class NoteLine(val start: Int, val text: String, val para: NSpan?, val spans: List<NSpan>, val number: Int)

/**
 * A note's lines with the spans on each, found in one pass (each span placed on the lines it
 * covers by binary search), so a note of thousands of lines and spans lays out in milliseconds.
 */
private fun lay(d: NDoc): List<NoteLine> {
  val texts = d.text.split('\n')
  val starts = IntArray(texts.size)
  var at = 0
  texts.forEachIndexed { i, s -> starts[i] = at; at += s.length + 1 }
  val on = Array(texts.size) { mutableListOf<NSpan>() }
  for (s in d.spans) {
    val from = s.start.toInt(); val to = (s.start + s.length).toInt()
    if (to <= from) continue
    var i = starts.binarySearch(from).let { if (it >= 0) it else -it - 2 }.coerceAtLeast(0)
    while (i < texts.size && starts[i] < to) { on[i].add(s); i++ }
  }
  var numbered = 0
  return texts.indices.map { i ->
    val para = on[i].firstOrNull { starts[i] >= it.start.toInt() && starts[i] < (it.start + it.length).toInt() }
    numbered = if (para?.style?.toInt() == 102) numbered + 1 else 0
    NoteLine(starts[i], texts[i], para, on[i], numbered)
  }
}

/**
 * A note drawn with its formatting, each line by its paragraph style, checklist items tickable;
 * lines composed only as they scroll into view.
 */
@Composable
private fun RichNote(d: NDoc, engine: AccountInterface, modifier: Modifier, onCheck: (UInt, Boolean) -> Unit) {
  val lines = remember(d) { lay(d) }
  LazyColumn(modifier, contentPadding = PaddingValues(horizontal = 24.dp, vertical = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
    items(lines.size) { i -> NoteLineView(lines[i], engine, onCheck) }
  }
}

@Composable
private fun NoteLineView(l: NoteLine, engine: AccountInterface, onCheck: (UInt, Boolean) -> Unit) {
  val para = l.para
  // The attachments on this line, in order: each U+FFFC is one, named by the span over it.
  val attachments = remember(l) {
    l.text.indices.filter { l.text[it] == '\uFFFC' }.mapNotNull { i ->
      val at = l.start + i
      l.spans.firstOrNull { it.attachment != null && at >= it.start.toInt() && at < (it.start + it.length).toInt() }?.attachment
    }
  }
  val style = when (para?.style?.toInt()) {
    0 -> MaterialTheme.typography.headlineMedium.copy(fontWeight = FontWeight.Bold)
    1 -> MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.Bold)
    2 -> MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold)
    4 -> MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace)
    else -> MaterialTheme.typography.bodyLarge
  }
  val link = MaterialTheme.colorScheme.primary
  val styled = buildAnnotatedString {
    // Each attachment's character kept as one that draws nothing, so every style after it stays
    // on its own words; the attachment itself is drawn below the line's words.
    append(l.text.replace('\uFFFC', '\u200B'))
    // Character formatting, stretch by stretch, clipped to this line.
    for (s in l.spans) {
      val from = maxOf(s.start.toInt(), l.start) - l.start
      val to = minOf((s.start + s.length).toInt(), l.start + l.text.length) - l.start
      if (to <= from) continue
      addStyle(
        SpanStyle(
          fontWeight = if (s.bold) FontWeight.Bold else null,
          fontStyle = if (s.italic) FontStyle.Italic else null,
          textDecoration = when {
            s.underline && s.strike -> TextDecoration.combine(listOf(TextDecoration.Underline, TextDecoration.LineThrough))
            s.underline || s.link != null -> TextDecoration.Underline
            s.strike -> TextDecoration.LineThrough
            else -> null
          },
          color = emphasisColour(s.emphasis) ?: if (s.link != null) link else Color.Unspecified,
        ),
        from, to,
      )
    }
  }
  val indent = (para?.indent?.toInt() ?: 0) * 20
  Row(Modifier.padding(start = indent.dp), verticalAlignment = Alignment.Top) {
    when (para?.style?.toInt()) {
      103 -> {
        // Notes' checklist circle: hollow, or filled yellow with a check.
        val ticked = para.checked == true
        val accent = MaterialTheme.colorScheme.primaryContainer
        Box(
          Modifier.padding(top = 3.dp, end = 10.dp).size(20.dp)
            .border(1.75.dp, if (ticked) accent else MaterialTheme.colorScheme.outline, androidx.compose.foundation.shape.CircleShape)
            .background(if (ticked) accent else Color.Transparent, androidx.compose.foundation.shape.CircleShape)
            .toggleable(ticked, role = Role.Checkbox) { onCheck(l.start.toUInt(), it) },
          contentAlignment = Alignment.Center,
        ) { if (ticked) Icon(painterResource(R.drawable.ic_check), contentDescription = null, tint = MaterialTheme.colorScheme.onPrimaryContainer, modifier = Modifier.size(13.dp)) }
      }
      100 -> Text("•  ", style = style)
      101 -> Text("–  ", style = style)
      102 -> Text("${l.number}.  ", style = style)
    }
    if (para?.quote == true) Box(Modifier.padding(end = 10.dp).size(3.dp, 22.dp).background(MaterialTheme.colorScheme.outline))
    // A line that is only its attachments shows them alone, without an empty line of text.
    if (attachments.isEmpty() || l.text.any { it != '\uFFFC' && !it.isWhitespace() }) {
      Text(styled, style = style.copy(lineHeight = (style.fontSize.value * 1.35f).sp))
    }
  }
  for (id in attachments) NoteAttachmentView(id, engine)
}

/**
 * One attachment in a note, as Notes shows it: a picture or drawing drawn in the note, a tag
 * or mention as its words, anything else (a PDF, a scan, a file) a card that opens in the app
 * the phone uses for it. Its kind is read once and kept by the engine; the picture is fetched
 * once into the cache.
 */
@Composable
private fun NoteAttachmentView(id: String, engine: AccountInterface) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  var info by remember(id) { mutableStateOf<NAttachment?>(null) }
  var picture by remember(id) { mutableStateOf<androidx.compose.ui.graphics.ImageBitmap?>(null) }
  var problem by remember(id) { mutableStateOf<String?>(null) }
  val key = id.filter { it.isLetterOrDigit() }
  LaunchedEffect(id) {
    runCatching { withContext(Dispatchers.IO) { engine.noteAttachment(id) } }
      .onSuccess { a ->
        info = a
        if (a.kind == "image" || a.kind == "drawing" || a.hasPreview) {
          picture = runCatching {
            withContext(Dispatchers.IO) {
              val file = java.io.File(context.cacheDir, "note-pictures/$key")
              // Apple's picture of it; a photo just added has none yet (Apple makes it later),
              // and a photo's own file is its picture.
              val got = runCatching { engine.noteAttachmentFile(id, NPart.PICTURE, file.path) }
              if (got.isFailure && a.kind == "image") engine.noteAttachmentFile(id, NPart.FILE, file.path)
              else got.getOrThrow()
              decodeScaled(file, 1600)
            }
          }.getOrNull()
        }
      }
      .onFailure { problem = "This attachment couldn't be read from iCloud." }
  }
  fun open(a: NAttachment) = scope.launch {
    runCatching {
      withContext(Dispatchers.IO) {
        // In the cache's share/ folder: the one place the app hands files to other apps from.
        val file = java.io.File(context.cacheDir, "share/note-files/$key/${a.name}")
        engine.noteAttachmentFile(id, NPart.FILE, file.path)
        file
      }
    }.onSuccess { file ->
      val uri = androidx.core.content.FileProvider.getUriForFile(context, "${context.packageName}.share", file)
      val type = android.webkit.MimeTypeMap.getSingleton()
        .getMimeTypeFromExtension(a.name.substringAfterLast('.', "").lowercase()) ?: "application/octet-stream"
      val view = android.content.Intent(android.content.Intent.ACTION_VIEW).setDataAndType(uri, type)
        .addFlags(android.content.Intent.FLAG_GRANT_READ_URI_PERMISSION)
      try {
        context.startActivity(android.content.Intent.createChooser(view, a.name).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK))
      } catch (_: android.content.ActivityNotFoundException) {
        problem = "No app on this phone opens ${a.name.substringAfterLast('.', "this kind of")} files."
      }
    }.onFailure { problem = "This attachment couldn't be downloaded from iCloud." }
  }
  val a = info
  val shape = RoundedCornerShape(12.dp)
  Column(Modifier.fillMaxWidth().padding(vertical = 6.dp)) {
    when {
      a == null && problem == null -> Box(Modifier.fillMaxWidth().size(height = 56.dp, width = 0.dp).background(MaterialTheme.colorScheme.surfaceVariant, shape))
      a == null -> {}
      a.kind == "inline" -> Text(a.title ?: "", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.primary)
      (a.kind == "image" || a.kind == "drawing") && picture != null -> {
        val p = picture!!
        androidx.compose.foundation.Image(
          p, contentDescription = a.name,
          modifier = Modifier.fillMaxWidth().aspectRatio(p.width.toFloat() / p.height.coerceAtLeast(1))
            .clip(shape).clickable { open(a) },
          contentScale = androidx.compose.ui.layout.ContentScale.Fit,
        )
      }
      else -> Row(
        Modifier.fillMaxWidth().clip(shape).background(MaterialTheme.colorScheme.surfaceVariant)
          .clickable(enabled = a.kind != "table") { open(a) }.padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        val p = picture
        if (p != null) androidx.compose.foundation.Image(
          p, contentDescription = null,
          modifier = Modifier.size(44.dp).clip(RoundedCornerShape(6.dp)),
          contentScale = androidx.compose.ui.layout.ContentScale.Crop,
        ) else Icon(
          painterResource(attachmentIcon(a)), contentDescription = null,
          tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(28.dp),
        )
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
          Text(if (a.kind == "table") "Table" else a.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
          val under = when {
            a.kind == "table" -> "Opens in Notes on an iPhone, iPad or Mac"
            else -> a.size?.let { android.text.format.Formatter.formatShortFileSize(context, it) }
          }
          if (under != null) Text(under, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
      }
    }
    problem?.let { Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.padding(top = 4.dp)) }
  }
}

/** The icon a file's card shows for what kind of file it is. */
private fun attachmentIcon(a: NAttachment): Int = when {
  a.kind == "audio" -> R.drawable.ic_file_audio
  a.kind == "video" -> R.drawable.ic_file_video
  a.kind == "image" || a.kind == "drawing" -> R.drawable.ic_file_image
  a.uti.contains("pdf") || a.kind == "document" -> R.drawable.ic_file_text
  a.uti.contains("spreadsheet") || a.uti.contains("numbers") -> R.drawable.ic_file_sheet
  a.uti.contains("presentation") || a.uti.contains("keynote") -> R.drawable.ic_file_slides
  else -> R.drawable.ic_file
}

/** A picture read at no more than `most` pixels a side, so a 48-megapixel photo never fills memory. */
private fun decodeScaled(file: java.io.File, most: Int): androidx.compose.ui.graphics.ImageBitmap? {
  val bounds = android.graphics.BitmapFactory.Options().apply { inJustDecodeBounds = true }
  android.graphics.BitmapFactory.decodeFile(file.path, bounds)
  var sample = 1
  while (bounds.outWidth / (sample * 2) >= most || bounds.outHeight / (sample * 2) >= most) sample *= 2
  val opts = android.graphics.BitmapFactory.Options().apply { inSampleSize = sample }
  return android.graphics.BitmapFactory.decodeFile(file.path, opts)?.asImageBitmap()
}

/**
 * The editor's view of a note's text: each attachment's character (U+FFFC, drawn by most fonts
 * as a dotted "OBJ" box) shown as 📎, two characters for one, with every cursor position mapped
 * across, so what is typed and saved is the note's own text, attachments in place.
 */
internal object AttachmentMarks : androidx.compose.ui.text.input.VisualTransformation {
  private const val MARK = "\uD83D\uDCCE"
  override fun filter(text: androidx.compose.ui.text.AnnotatedString): androidx.compose.ui.text.input.TransformedText {
    val raw = text.text
    if ('\uFFFC' !in raw) return androidx.compose.ui.text.input.TransformedText(text, androidx.compose.ui.text.input.OffsetMapping.Identity)
    val marks = raw.indices.filter { raw[it] == '\uFFFC' }
    val shown = raw.replace("\uFFFC", MARK)
    val mapping = object : androidx.compose.ui.text.input.OffsetMapping {
      override fun originalToTransformed(offset: Int) = offset + marks.count { it < offset }
      override fun transformedToOriginal(offset: Int): Int {
        var extra = 0
        for ((k, m) in marks.withIndex()) {
          // The k-th mark is shown from m + k, two places wide; a place inside it is before it.
          val shownAt = m + k
          when {
            offset >= shownAt + 2 -> extra++
            offset == shownAt + 1 -> return m
            else -> break
          }
        }
        return (offset - extra).coerceIn(0, raw.length)
      }
    }
    return androidx.compose.ui.text.input.TransformedText(androidx.compose.ui.text.AnnotatedString(shown), mapping)
  }
}

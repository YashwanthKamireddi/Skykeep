package io.github.yashwanthkamireddi.skykeep.ui

import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import android.content.ContentResolver
import android.os.Bundle
import androidx.activity.compose.BackHandler
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
import uniffi.quince_mobile.NDoc
import uniffi.quince_mobile.NFolder
import uniffi.quince_mobile.NItem
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
fun NotesPage(engine: Account) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  var place by rememberSaveable(stateSaver = NPlaceSaver) { mutableStateOf<NPlace>(NPlace.Folders) }
  var folders by remember { mutableStateOf<List<NFolder>>(emptyList()) }
  var notes by remember { mutableStateOf<List<NItem>>(emptyList()) }
  var query by remember { mutableStateOf("") }
  var problem by remember { mutableStateOf<String?>(null) }
  var generation by remember { mutableIntStateOf(0) }

  fun reason(t: Throwable) = (t as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "Notes couldn't be read."

  fun reload() = scope.launch {
    val p = place
    runCatching {
      withContext(Dispatchers.IO) {
        val f = engine.noteFolders()
        val n = when {
          query.isNotBlank() -> engine.searchNotes(query)
          p is NPlace.Folder -> engine.notes(p.guid)
          else -> emptyList()
        }
        f to n
      }
    }.onSuccess { (f, n) -> folders = f; notes = n; problem = null }.onFailure { problem = reason(it) }
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
      is NPlace.Open -> NoteScreen(engine, p.guid, p.from.guid, onBack = { place = p.from; generation++ }, onProblem = { problem = it })
      else -> {
        val folder = p as? NPlace.Folder
        Row(Modifier.padding(start = 4.dp, end = 12.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
          if (folder != null) IconButton(onClick = { place = NPlace.Folders; query = "" }) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Back") }
          else Spacer(Modifier.width(16.dp))
          Text(folder?.title ?: "Notes", style = MaterialTheme.typography.headlineMedium, modifier = Modifier.weight(1f))
          if (folder != null && folders.firstOrNull { it.guid == folder.guid }?.trash != true) {
            // Nothing is made yet: a new note is made in iCloud when something is typed in it,
            // so one opened and left empty leaves nothing behind, as in Notes.
            TextButton(onClick = { place = NPlace.Open(null, folder) }) { Text("New Note", color = NotesYellow) }
          }
        }
        OutlinedTextField(
          query, { query = it }, placeholder = { Text("Search") }, singleLine = true,
          shape = RoundedCornerShape(16.dp), modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        )
        if (folder == null && query.isBlank()) {
          FolderList(folders) { place = it }
        } else {
          NoteList(notes) { n -> place = NPlace.Open(n.guid, folder ?: NPlace.Folder(null, "All Notes")) }
        }
      }
    }
  }
}

@Composable
private fun FolderList(folders: List<NFolder>, open: (NPlace.Folder) -> Unit) {
  val all = folders.filter { !it.trash }.sumOf { it.count.toInt() }
  LazyColumn(contentPadding = PaddingValues(horizontal = 20.dp, vertical = 8.dp)) {
    item { FolderRow("All Notes", all) { open(NPlace.Folder(null, "All Notes")) } }
    // Apple's own "Notes" folder is the one with no folder: notes not filed anywhere.
    items(folders.filter { !it.trash }, key = { it.guid }) { f -> FolderRow(f.title, f.count.toInt()) { open(NPlace.Folder(f.guid, f.title)) } }
    folders.firstOrNull { it.trash }?.let { t ->
      item { FolderRow("Recently Deleted", t.count.toInt()) { open(NPlace.Folder(t.guid, "Recently Deleted")) } }
    }
  }
}

@Composable
private fun FolderRow(title: String, count: Int, onClick: () -> Unit) {
  Row(Modifier.fillMaxWidth().clickable(onClick = onClick).padding(vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
    Text(title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f), maxLines = 1, overflow = TextOverflow.Ellipsis)
    Text("$count", style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
  }
  HorizontalDivider()
}

private val dayFormat: DateTimeFormatter = DateTimeFormatter.ofLocalizedDate(FormatStyle.SHORT)
private val timeFormat: DateTimeFormatter = DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT)

/** When a note was changed, as Notes says it: the time today, else the date. */
private fun changed(at: Long?): String {
  at ?: return ""
  val z = Instant.ofEpochSecond(at).atZone(ZoneId.systemDefault())
  return if (z.toLocalDate() == LocalDate.now()) z.toLocalTime().format(timeFormat) else z.toLocalDate().format(dayFormat)
}

@Composable
private fun NoteList(notes: List<NItem>, open: (NItem) -> Unit) {
  LazyColumn(contentPadding = PaddingValues(horizontal = 20.dp, vertical = 4.dp)) {
    if (notes.isEmpty()) item { Text("No notes", color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(vertical = 32.dp)) }
    items(notes, key = { it.guid }) { n ->
      Column(Modifier.fillMaxWidth().clickable { open(n) }.padding(vertical = 12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
          if (n.pinned) Text("Pinned · ", style = MaterialTheme.typography.labelMedium, color = NotesYellow)
          Text(n.title.ifBlank { "New Note" }, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        }
        Row {
          Text(changed(n.modifiedAt), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
          Spacer(Modifier.width(8.dp))
          Text(
            if (n.locked) "Locked" else n.snippet?.lineSequence()?.firstOrNull { it.isNotBlank() } ?: "No additional text",
            style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis,
          )
        }
      }
      HorizontalDivider()
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
private fun NoteScreen(engine: Account, opened: String?, folderGuid: String?, onBack: () -> Unit, onProblem: (String) -> Unit) {
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
  val d = doc ?: NDoc(guid ?: "", folderGuid, "", text, emptyList(), pinned = false, locked = false, editable = true).takeIf { guid == null } ?: return
  Column(Modifier.fillMaxSize()) {
    Row(Modifier.padding(start = 4.dp, end = 8.dp, top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
      IconButton(onClick = { save { onBack() } }) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Back") }
      Spacer(Modifier.weight(1f))
      val g = guid
      // In Recently Deleted a note is recovered or deleted for good, as in Notes; not pinned or edited.
      val trashed = d.folderGuid == TRASH
      if (!d.locked && g != null) {
        if (trashed) {
          TextButton(onClick = { scope.launch { withContext(Dispatchers.IO) { runCatching { engine.noteMove(g, null) } }; syncNotes(context); onBack() } }) {
            Text("Recover", color = NotesYellow)
          }
        } else {
          TextButton(onClick = { scope.launch { withContext(Dispatchers.IO) { runCatching { engine.notePin(g, !d.pinned) } }; syncNotes(context); load() } }) {
            Text(if (d.pinned) "Unpin" else "Pin", color = NotesYellow)
          }
        }
        TextButton(onClick = { scope.launch { withContext(Dispatchers.IO) { runCatching { engine.noteDelete(g) } }; syncNotes(context); onBack() } }) {
          Text(if (trashed) "Delete Now" else "Delete", color = MaterialTheme.colorScheme.error)
        }
      }
      if (!d.locked) {
        if (editing) TextButton(onClick = { save { editing = false; load() } }) { Text("Done", color = NotesYellow) }
      }
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
          textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
          cursorBrush = SolidColor(NotesYellow),
          modifier = Modifier.fillMaxSize().imePadding().padding(horizontal = 24.dp, vertical = 8.dp).focusRequester(focus),
        )
      }
      else -> Box(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
          .clickable(enabled = d.editable && d.folderGuid != TRASH) { text = d.text; saved = d.text; editing = true }
          .padding(horizontal = 24.dp, vertical = 8.dp),
      ) {
        RichNote(d) { at, checked ->
          val g = guid ?: return@RichNote
          scope.launch {
            runCatching { withContext(Dispatchers.IO) { engine.noteCheck(g, at, checked) } }.onFailure { onProblem(reason(it)) }
            syncNotes(context); load()
          }
        }
      }
    }
  }
}

/** A note drawn with its formatting: each line by its paragraph style, checklist items tickable. */
@Composable
private fun RichNote(d: NDoc, onCheck: (UInt, Boolean) -> Unit) {
  // Lines with where each starts (UTF-16, as the spans count), and the span setting each.
  var at = 0
  val lines = d.text.split('\n').map { line -> (at to line).also { at += line.length + 1 } }
  fun spanAt(pos: Int): NSpan? = d.spans.firstOrNull { pos >= it.start.toInt() && pos < (it.start + it.length).toInt() }
  Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
    for ((start, line) in lines) {
      val para = spanAt(start)
      val style = when (para?.style?.toInt()) {
        0 -> MaterialTheme.typography.headlineMedium.copy(fontWeight = FontWeight.Bold)
        1 -> MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.Bold)
        2 -> MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold)
        4 -> MaterialTheme.typography.bodyMedium.copy(fontFamily = FontFamily.Monospace)
        else -> MaterialTheme.typography.bodyLarge
      }
      val styled = buildAnnotatedString {
        append(line.replace("￼", "[attachment]"))
        // Character formatting, stretch by stretch, clipped to this line.
        for (s in d.spans) {
          val from = maxOf(s.start.toInt(), start) - start
          val to = minOf((s.start + s.length).toInt(), start + line.length) - start
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
              color = emphasisColour(s.emphasis) ?: if (s.link != null) MaterialTheme.colorScheme.primary else Color.Unspecified,
            ),
            from, to,
          )
        }
      }
      val indent = (para?.indent?.toInt() ?: 0) * 20
      Row(Modifier.padding(start = indent.dp), verticalAlignment = Alignment.Top) {
        when (para?.style?.toInt()) {
          103 -> {
            val ticked = para.checked == true
            Box(
              Modifier.padding(top = 3.dp, end = 10.dp).size(20.dp)
                .border(2.dp, if (ticked) NotesYellow else MaterialTheme.colorScheme.outline, androidx.compose.foundation.shape.CircleShape)
                .background(if (ticked) NotesYellow else Color.Transparent, androidx.compose.foundation.shape.CircleShape)
                .clickable { onCheck(start.toUInt(), !ticked) },
            )
          }
          100 -> Text("•  ", style = style)
          101 -> Text("–  ", style = style)
          102 -> Text("${lines.takeWhile { it.first < start }.count { spanAt(it.first)?.style?.toInt() == 102 } + 1}.  ", style = style)
        }
        if (para?.quote == true) Box(Modifier.padding(end = 10.dp).width(3.dp).size(3.dp, 22.dp).background(MaterialTheme.colorScheme.outline))
        Text(styled, style = style.copy(lineHeight = (style.fontSize.value * 1.35f).sp))
      }
    }
  }
}

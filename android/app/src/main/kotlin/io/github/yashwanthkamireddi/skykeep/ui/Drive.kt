package io.github.yashwanthkamireddi.skykeep.ui

import android.content.ActivityNotFoundException
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract
import android.provider.OpenableColumns
import android.text.format.DateUtils
import android.text.format.Formatter
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.sync.DriveFiles
import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import io.github.yashwanthkamireddi.skykeep.ui.theme.mark
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.AccountInterface
import uniffi.quince_mobile.DItem
import uniffi.quince_mobile.EngineException

/** A folder on the way down from the top: its id and name. */
private data class Crumb(val id: String, val name: String)

/** The mark for a file, by its kind. */
private fun iconOf(item: DItem): Int {
  if (item.folder) return R.drawable.ic_folder
  val mime = DriveFiles.mimeOf(item)
  val ext = item.name.substringAfterLast('.', "").lowercase()
  return when {
    mime.startsWith("image/") -> R.drawable.ic_file_image
    mime.startsWith("video/") -> R.drawable.ic_file_video
    mime.startsWith("audio/") -> R.drawable.ic_file_audio
    ext in setOf("zip", "gz", "tar", "7z", "rar", "xz") -> R.drawable.ic_file_archive
    ext in setOf("numbers", "xlsx", "xls", "csv", "ods") -> R.drawable.ic_file_sheet
    ext in setOf("key", "pptx", "ppt", "odp") -> R.drawable.ic_file_slides
    mime.startsWith("text/") || ext in setOf("pdf", "pages", "docx", "doc", "rtf", "md", "odt") -> R.drawable.ic_file_text
    else -> R.drawable.ic_file
  }
}

private fun reason(t: Throwable, fallback: String) = (t as? EngineException.Failed)?.reason ?: fallback

/**
 * iCloud Drive inside Skykeep, as the desktop's Drive app has it: the folders and files of each
 * folder (from the phone's library at once, then fresh from Apple), how much of iCloud is used,
 * search, and every file opened in the app the phone uses for its kind. Upload, new folder,
 * rename and delete show at once and are undone, with the reason, if Apple refuses. The same
 * files are in the Files app (DriveProvider) and every app's file picker.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DrivePage(engine: AccountInterface) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  val root = remember { engine.driveRoot() }
  val path = remember { mutableStateListOf(Crumb(root, "iCloud Drive")) }
  val here = path.last()
  var items by remember { mutableStateOf<List<DItem>>(emptyList()) }
  var loaded by remember { mutableStateOf(false) }
  var refreshing by remember { mutableStateOf(false) }
  var space by remember { mutableStateOf<List<ULong>?>(null) }
  var query by rememberSaveable { mutableStateOf("") }
  var results by remember { mutableStateOf<List<DItem>>(emptyList()) }
  var problem by remember { mutableStateOf<String?>(null) }
  var naming by remember { mutableStateOf<DItem?>(null) }
  var newFolder by remember { mutableStateOf(false) }
  /** Names on their way up: shown in the folder at once, until Apple lists them. */
  val uploading = remember { mutableStateListOf<Pair<String, String>>() }
  var generation by remember { mutableIntStateOf(0) }

  fun load(fresh: Boolean) = scope.launch {
    val folder = here.id
    runCatching { withContext(Dispatchers.IO) { engine.driveChildren(folder, fresh) } }
      .onSuccess { if (path.last().id == folder) { items = it; problem = null } }
      .onFailure { problem = reason(it, "iCloud Drive couldn't be read.") }
    loaded = true
  }

  LaunchedEffect(here.id, generation) {
    loaded = false
    // What the phone has first, at once; then Apple's, behind.
    load(false).join()
    load(true)
  }
  LaunchedEffect(Unit) {
    runCatching { withContext(Dispatchers.IO) { engine.driveSpace() } }.onSuccess { space = it.map { v -> v.toULong() } }
  }
  LaunchedEffect(query) {
    results = if (query.isBlank()) emptyList()
    else runCatching { withContext(Dispatchers.IO) { engine.driveSearch(query) } }.getOrDefault(emptyList())
  }
  BackHandler(enabled = path.size > 1 || query.isNotEmpty()) { if (query.isNotEmpty()) query = "" else path.removeAt(path.lastIndex) }

  /** A change sent to Apple; the folder is read again either way, and a refusal is said. */
  fun change(what: String, write: () -> Unit) = scope.launch {
    runCatching { withContext(Dispatchers.IO) { write() } }.onFailure { problem = reason(it, "$what didn't reach iCloud.") }
    load(false)
  }

  fun open(item: DItem) {
    if (item.folder) { query = ""; path.add(Crumb(item.id, item.name)); return }
    scope.launch {
      // Handed to the app that opens it through iCloud Drive's own provider, which serves the
      // file as that app reads it (a film plays, a long PDF opens, before it has all arrived).
      val uri = DocumentsContract.buildDocumentUri("${context.packageName}.drive", item.id)
      val view = Intent(Intent.ACTION_VIEW).setDataAndType(uri, DriveFiles.mimeOf(item))
        .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
      try {
        context.startActivity(Intent.createChooser(view, item.name).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
      } catch (_: ActivityNotFoundException) {
        problem = "No app on this phone opens ${item.name.substringAfterLast('.', "this kind of file")} files."
      }
    }
  }

  val pick = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris: List<Uri> ->
    val folder = here.id
    for (uri in uris) {
      val name = context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
        ?.use { c -> if (c.moveToFirst()) c.getString(0) else null } ?: "Untitled"
      uploading += folder to name
      scope.launch {
        runCatching {
          withContext(Dispatchers.IO) {
            // Staged under its own name: the name is what iCloud Drive calls it.
            val staged = File(context.cacheDir, "drive-upload/${System.nanoTime()}/${DriveFiles.safeName(name)}").also { it.parentFile?.mkdirs() }
            context.contentResolver.openInputStream(uri)!!.use { input -> staged.outputStream().use { input.copyTo(it) } }
            try { engine.driveWriteBack(folder, staged.path) } finally { staged.parentFile?.deleteRecursively() }
          }
        }.onFailure { problem = reason(it, "$name couldn't be uploaded.") }
        uploading -= folder to name
        if (path.last().id == folder) load(false)
      }
    }
  }

  Column(Modifier.fillMaxSize()) {
    Row(Modifier.padding(start = 4.dp, end = 4.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
      if (path.size > 1) IconButton(onClick = { path.removeAt(path.lastIndex) }) {
        Icon(painterResource(R.drawable.ic_back), contentDescription = path[path.lastIndex - 1].name)
      }
      Spacer(Modifier.weight(1f))
      IconButton(onClick = { newFolder = true }) { Icon(painterResource(R.drawable.ic_folder_plus), contentDescription = "New Folder", tint = MaterialTheme.colorScheme.primary) }
      IconButton(onClick = { pick.launch(arrayOf("*/*")) }) { Icon(painterResource(R.drawable.ic_upload), contentDescription = "Upload", tint = MaterialTheme.colorScheme.primary) }
    }
    Column(Modifier.padding(horizontal = 20.dp)) {
      Text(here.name, style = MaterialTheme.typography.headlineLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
      if (query.isBlank() && loaded) {
        val n = items.size
        Text(if (n == 1) "1 item" else "$n items", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
      }
    }
    SearchPill(query, { query = it }, Modifier.padding(horizontal = 16.dp, vertical = 12.dp))
    problem?.let {
      Box(Modifier.padding(horizontal = 16.dp).padding(bottom = 8.dp)) { Notice(it, warning = true) { TextButton(onClick = { problem = null }) { Text("OK") } } }
    }
    PullToRefreshBox(
      isRefreshing = refreshing,
      onRefresh = { scope.launch { refreshing = true; load(true).join(); refreshing = false } },
      modifier = Modifier.weight(1f),
    ) {
      LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        if (path.size == 1 && query.isBlank()) space?.let { (used, total) ->
          item { Storage(used, total) }
        }
        val shown = if (query.isNotBlank()) results else
          items.sortedWith(compareBy<DItem>({ !it.folder }, { it.name.lowercase() }))
        val pending = if (query.isBlank()) uploading.filter { it.first == here.id }.map { it.second } else emptyList()
        if (shown.isEmpty() && pending.isEmpty() && (loaded || query.isNotBlank())) item {
          Text(
            if (query.isNotBlank()) "No results for “$query”" else "This folder is empty",
            style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 4.dp, vertical = 24.dp),
          )
        }
        if (shown.isNotEmpty() || pending.isNotEmpty()) item {
          GroupCard(pending.map { null to it } + shown.map { it to it.name }, inset = 56) { (item, name) ->
            if (item == null) UploadingRow(name)
            else DriveRow(
              item,
              onOpen = { open(item) },
              onRename = { naming = item },
              onDelete = {
                items = items.filter { it.id != item.id }
                change("Deleting ${item.name}") { engine.driveDelete(item.id) }
              },
            )
          }
        }
      }
    }
  }

  naming?.let { item ->
    NameDialog("Rename", item.name, onDismiss = { naming = null }) { name ->
      naming = null
      if (name != item.name) {
        items = items.map { if (it.id == item.id) it.copy(name = name) else it }
        change("Renaming ${item.name}") { engine.driveRename(item.id, name) }
      }
    }
  }
  if (newFolder) NameDialog("New Folder", "", onDismiss = { newFolder = false }) { name ->
    newFolder = false
    val folder = here.id
    change("Making $name") { engine.driveMkdir(folder, name) }
  }
}

/** How much of the person's iCloud is used: a determinate bar and the numbers. */
@Composable
private fun Storage(used: ULong, total: ULong) {
  val context = LocalContext.current
  val fraction = if (total > 0u) (used.toDouble() / total.toDouble()).toFloat().coerceIn(0f, 1f) else 0f
  androidx.compose.material3.Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large) {
    Column(Modifier.fillMaxWidth().padding(16.dp).semantics(mergeDescendants = true) {}, verticalArrangement = Arrangement.spacedBy(10.dp)) {
      Text(
        "${Formatter.formatShortFileSize(context, used.toLong())} of ${Formatter.formatShortFileSize(context, total.toLong())} used",
        style = MaterialTheme.typography.bodyMedium,
      )
      LinearProgressIndicator(
        progress = { fraction }, modifier = Modifier.fillMaxWidth(),
        color = mark(Palette.App.Drive), trackColor = MaterialTheme.colorScheme.surfaceContainerHighest,
        drawStopIndicator = {},
      )
    }
  }
}

@Composable
private fun DriveRow(item: DItem, onOpen: () -> Unit, onRename: () -> Unit, onDelete: () -> Unit) {
  val context = LocalContext.current
  var menu by remember { mutableStateOf(false) }
  Row(
    Modifier.fillMaxWidth().clickable(onClick = onOpen).padding(start = 16.dp, end = 4.dp, top = 10.dp, bottom = 10.dp),
    verticalAlignment = Alignment.CenterVertically,
  ) {
    Icon(
      painterResource(iconOf(item)), contentDescription = null,
      tint = if (item.folder) mark(Palette.App.Drive) else MaterialTheme.colorScheme.onSurfaceVariant,
      modifier = Modifier.size(24.dp),
    )
    Spacer(Modifier.width(16.dp))
    Column(Modifier.weight(1f)) {
      Text(item.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
      val detail = if (item.folder) item.children?.let { if (it == 1L) "1 item" else "$it items" }
      else listOfNotNull(
        item.modifiedAt?.let { DateUtils.getRelativeTimeSpanString(it * 1000, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString() },
        item.size?.let { Formatter.formatShortFileSize(context, it) },
      ).joinToString(" · ").ifEmpty { null }
      detail?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant) }
    }
    Box {
      IconButton(onClick = { menu = true }) { Icon(painterResource(R.drawable.ic_more), contentDescription = "More for ${item.name}", tint = MaterialTheme.colorScheme.onSurfaceVariant) }
      DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
        DropdownMenuItem(text = { Text("Rename") }, onClick = { menu = false; onRename() })
        DropdownMenuItem(text = { Text("Delete", color = MaterialTheme.colorScheme.error) }, onClick = { menu = false; onDelete() })
      }
    }
  }
}

@Composable
private fun UploadingRow(name: String) {
  Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
    Icon(painterResource(R.drawable.ic_upload), contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(24.dp))
    Spacer(Modifier.width(16.dp))
    Column(Modifier.weight(1f)) {
      Text(name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
      Text("Uploading", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
    CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
  }
}

/** A name asked for: a folder's, or a new name for an item. Names Apple can't hold are refused here. */
@Composable
internal fun NameDialog(title: String, initial: String, onDismiss: () -> Unit, onDone: (String) -> Unit) {
  var name by remember { mutableStateOf(initial) }
  val clean = name.trim()
  // The keyboard goes with the dialog, whichever way it closes.
  val keyboard = androidx.compose.ui.platform.LocalSoftwareKeyboardController.current
  val close = { keyboard?.hide(); onDismiss() }
  val ok = clean.isNotEmpty() && '/' !in clean && clean != "." && clean != ".."
  AlertDialog(
    onDismissRequest = close,
    title = { Text(title) },
    text = { OutlinedTextField(name, { name = it }, singleLine = true, label = { Text("Name") }, isError = name.isNotEmpty() && !ok) },
    confirmButton = { TextButton(onClick = { keyboard?.hide(); onDone(clean) }, enabled = ok) { Text("Done") } },
    dismissButton = { TextButton(onClick = close) { Text("Cancel") } },
  )
}

package io.github.yashwanthkamireddi.skykeep.ui

import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.Bundle
import android.os.CancellationSignal
import android.util.LruCache
import android.widget.VideoView
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.SnackbarResult
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.FileProvider
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.sync.Outcome
import io.github.yashwanthkamireddi.skykeep.sync.PHOTOS_AUTHORITY
import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import io.github.yashwanthkamireddi.skykeep.ui.theme.mark
import java.io.File
import java.time.Instant
import java.time.YearMonth
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.time.format.TextStyle
import java.util.Locale
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.AccountInterface
import uniffi.quince_mobile.EngineException
import uniffi.quince_mobile.PAlbum
import uniffi.quince_mobile.PFlag
import uniffi.quince_mobile.PIndex
import uniffi.quince_mobile.PLibrary
import uniffi.quince_mobile.PLibraryChoice
import uniffi.quince_mobile.PSharedAlbum
import uniffi.quince_mobile.PSize
import uniffi.quince_mobile.PView

/** Thumbnails decoded, the most recently shown kept: a grid scrolls back without decoding again. */
private val thumbs = object : LruCache<String, ImageBitmap>(48 * 1024 * 1024) {
  override fun sizeOf(key: String, value: ImageBitmap) = value.width * value.height * 4
}

/** A few pictures fetched at once: a grid flung fast asks for hundreds. */
private val fetching = Semaphore(6)

private fun syncPhotos(context: Context) {
  phoneAccount(context)?.let {
    ContentResolver.requestSync(it, PHOTOS_AUTHORITY, Bundle().apply {
      putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
      putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
    })
  }
}

/** Where a photo's picture of a size is kept on the phone. */
private fun cached(context: Context, id: String, size: String): File =
  File(context.cacheDir, "photos/$size/${io.github.yashwanthkamireddi.skykeep.sync.DriveFiles.keyOf(id)}")

/** A picture decoded no larger than `long` pixels on its long side. */
private fun decode(file: File, long: Int): ImageBitmap? {
  val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
  BitmapFactory.decodeFile(file.path, bounds)
  if (bounds.outWidth <= 0) return null
  var sample = 1
  while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= long) sample *= 2
  return BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample })?.asImageBitmap()
}

/** A photo's thumbnail: decoded from memory, the phone's copy, or fetched once from Apple. */
private suspend fun thumbnail(context: Context, engine: AccountInterface, id: String, px: Int): ImageBitmap? {
  thumbs.get(id)?.let { return it }
  return withContext(Dispatchers.IO) {
    fetching.withPermit {
      val file = cached(context, id, "thumb")
      runCatching { if (file.exists() || engine.photoFile(id, PSize.THUMB, file.path)) decode(file, px.coerceAtLeast(120)) else null }.getOrNull()
    }
  }?.also { thumbs.put(id, it) }
}

/** The places Photos has besides the library, as the desktop's sidebar lists them. */
private data class PhotoPlace(val key: String, val title: String, val icon: Int, val view: PView)

private val PLACES = listOf(
  PhotoPlace("library", "Library", R.drawable.ic_photos, PView.Library),
  PhotoPlace("favorites", "Favorites", R.drawable.ic_heart, PView.Favorites),
  PhotoPlace("recents", "Recents", R.drawable.ic_clock, PView.Recents),
  PhotoPlace("videos", "Videos", R.drawable.ic_video, PView.Videos),
  PhotoPlace("screenshots", "Screenshots", R.drawable.ic_screenshot, PView.Screenshots),
  PhotoPlace("panoramas", "Panoramas", R.drawable.ic_panorama, PView.Panoramas),
  PhotoPlace("portrait", "Portrait", R.drawable.ic_portrait, PView.Portrait),
  PhotoPlace("hidden", "Hidden", R.drawable.ic_eye_off, PView.Hidden),
  PhotoPlace("deleted", "Recently Deleted", R.drawable.ic_trash, PView.RecentlyDeleted),
)

private fun placeOf(key: String, albums: List<PAlbum>, shared: List<PSharedAlbum>): PhotoPlace =
  PLACES.firstOrNull { it.key == key }
    ?: if (key.startsWith(SHARED)) key.removePrefix(SHARED).let { zone ->
      PhotoPlace(key, shared.firstOrNull { it.zone == zone }?.title ?: "Shared Album", R.drawable.ic_albums, PView.Shared(zone))
    } else key.removePrefix("album:").let { id -> PhotoPlace(key, albums.firstOrNull { it.id == id }?.name ?: "Album", R.drawable.ic_albums, PView.Album(id)) }

/** A shared album's key: other people's photos too, looked at and shared from here, never changed. */
private const val SHARED = "shared:"

/** The library switcher's choices, in the desktop's words. */
private val LIBRARIES = listOf(PLibrary.BOTH to "Both Libraries", PLibrary.PERSONAL to "Personal Library", PLibrary.SHARED to "Shared Library")

/** One row of the grid: a month's heading, or a photo by its place in the index. */
private sealed interface Cell {
  data class Month(val label: String, val first: Int) : Cell
  data class Photo(val at: Int) : Cell
}

/** The grid's rows: each month's heading before its photos, in the index's order. */
private fun cells(idx: PIndex): List<Cell> {
  val zone = ZoneId.systemDefault()
  val out = ArrayList<Cell>(idx.ids.size + idx.ids.size / 20 + 1)
  var month: YearMonth? = null
  for (i in idx.ids.indices) {
    val m = YearMonth.from(Instant.ofEpochSecond(idx.taken[i]).atZone(zone))
    if (m != month) {
      month = m
      out += Cell.Month("${m.month.getDisplayName(TextStyle.FULL, Locale.getDefault())} ${m.year}", i)
    }
    out += Cell.Photo(i)
  }
  return out
}

/** "July – October 2026", the span a grid covers, as the desktop's header says it. */
private fun span(idx: PIndex): String? {
  if (idx.taken.isEmpty()) return null
  val zone = ZoneId.systemDefault()
  val a = YearMonth.from(Instant.ofEpochSecond(idx.taken.min()).atZone(zone))
  val b = YearMonth.from(Instant.ofEpochSecond(idx.taken.max()).atZone(zone))
  fun name(m: YearMonth) = m.month.getDisplayName(TextStyle.FULL, Locale.getDefault())
  return when {
    a == b -> "${name(a)} ${a.year}"
    a.year == b.year -> "${name(a)} – ${name(b)} ${b.year}"
    else -> "${name(a)} ${a.year} – ${name(b)} ${b.year}"
  }
}

/** The person proves it is them (fingerprint, face, or the phone's screen lock) before Hidden opens. */
private fun unlock(context: Context, onOk: () -> Unit) {
  val prompt = BiometricPrompt.Builder(context).setTitle("Show Hidden").setSubtitle("Hidden photos stay hidden until you unlock them.")
  if (Build.VERSION.SDK_INT >= 30) {
    prompt.setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG or BiometricManager.Authenticators.DEVICE_CREDENTIAL)
  } else {
    @Suppress("DEPRECATION")
    prompt.setDeviceCredentialAllowed(true)
  }
  prompt.build().authenticate(CancellationSignal(), context.mainExecutor, object : BiometricPrompt.AuthenticationCallback() {
    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult?) = onOk()
  })
}

/** Photos handed to another app: each fetched at the viewer's size and shared as one copy. */
private suspend fun share(context: Context, engine: AccountInterface, ids: List<String>) {
  val uris = withContext(Dispatchers.IO) {
    // Copies shared before stay a day (the app they went to may still be reading), then go.
    val dayAgo = System.currentTimeMillis() - 86_400_000
    File(context.cacheDir, "share").listFiles()?.filter { it.lastModified() < dayAgo }?.forEach { it.deleteRecursively() }
    ids.mapNotNull { id ->
      val item = runCatching { engine.photoItem(id) }.getOrNull() ?: return@mapNotNull null
      val base = item.filename?.substringBeforeLast('.') ?: "photo"
      val name = if (item.video) "$base.mp4" else "$base.jpg"
      val file = File(context.cacheDir, "share/${io.github.yashwanthkamireddi.skykeep.sync.DriveFiles.keyOf(id)}/${io.github.yashwanthkamireddi.skykeep.sync.DriveFiles.safeName(name)}").also { it.parentFile?.mkdirs() }
      val size = if (item.video) PSize.VIDEO else PSize.MEDIUM
      if (!file.exists() && !runCatching { engine.photoFile(id, size, file.path) }.getOrDefault(false)) return@mapNotNull null
      FileProvider.getUriForFile(context, "${context.packageName}.share", file)
    }
  }
  if (uris.isEmpty()) return
  val send = if (uris.size == 1) Intent(Intent.ACTION_SEND).putExtra(Intent.EXTRA_STREAM, uris[0]).setType("*/*")
  else Intent(Intent.ACTION_SEND_MULTIPLE).putParcelableArrayListExtra(Intent.EXTRA_STREAM, ArrayList(uris)).setType("*/*")
  send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
  context.startActivity(Intent.createChooser(send, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

/**
 * Photos, as the desktop's Photos app has it: the library newest first, month by month, from one
 * compact index (a library of any size opens at once); pinch for more or fewer photos a row; a
 * date scrubber for a long library; Collections for Favorites, Recents, Videos and the rest,
 * Hidden behind the phone's own lock, Recently Deleted and the albums. Hold a photo to select;
 * Favorite, Hide, Delete (with Undo) and Share are shown at once and sent behind. Thumbnails are
 * fetched once as they come into view and kept.
 */
@Composable
fun PhotosPage(engine: AccountInterface) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  var key by rememberSaveable { mutableStateOf("library") }
  var collections by rememberSaveable { mutableStateOf(false) }
  var hiddenOpen by remember { mutableStateOf(false) }
  var albums by remember { mutableStateOf<List<PAlbum>>(emptyList()) }
  var shared by remember { mutableStateOf<List<PSharedAlbum>>(emptyList()) }
  var library by remember { mutableStateOf<PLibraryChoice?>(null) }
  var index by remember { mutableStateOf<PIndex?>(null) }
  var open by rememberSaveable { mutableStateOf<Int?>(null) }
  var columns by rememberSaveable { mutableIntStateOf(4) }
  var selected by remember { mutableStateOf<Set<String>>(emptySet()) }
  var generation by remember { mutableIntStateOf(0) }
  val snackbar = remember { SnackbarHostState() }
  // Hidden never opens without the person proving it is them in this sitting.
  if (key == "hidden" && !hiddenOpen) key = "library"
  val place = placeOf(key, albums, shared)
  val readOnly = key.startsWith(SHARED)

  // Hidden, and Recently Deleted (which can hold hidden photos), stay out of the Recents
  // thumbnail and of screenshots while shown, a photo opened from them included.
  val window = remember(context) {
    generateSequence(context) { (it as? android.content.ContextWrapper)?.baseContext }
      .filterIsInstance<android.app.Activity>().firstOrNull()?.window
  }
  val sensitive = key == "hidden" || key == "deleted"
  DisposableEffect(sensitive, window) {
    if (sensitive) window?.addFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE)
    onDispose { window?.clearFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE) }
  }
  // Hidden locks again whenever Skykeep leaves the screen: unlocking holds for this sitting only.
  val lifecycle = androidx.lifecycle.compose.LocalLifecycleOwner.current.lifecycle
  DisposableEffect(lifecycle) {
    val watch = androidx.lifecycle.LifecycleEventObserver { _, event ->
      if (event == androidx.lifecycle.Lifecycle.Event.ON_STOP) {
        hiddenOpen = false
        if (key == "hidden") { open = null; key = "library" }
      }
    }
    lifecycle.addObserver(watch)
    onDispose { lifecycle.removeObserver(watch) }
  }

  LaunchedEffect(Unit) { syncPhotos(context) }
  DisposableEffect(Unit) {
    val stop = Outcome.watch(context) { generation++ }
    onDispose { stop() }
  }
  LaunchedEffect(key, generation) {
    withContext(Dispatchers.IO) {
      runCatching { engine.photoIndex(place.view) }.getOrNull()?.let { index = it }
      runCatching { engine.photoAlbums() }.getOrNull()?.let { albums = it.filter { a -> !a.folder } }
      runCatching { engine.photoSharedAlbums() }.getOrNull()?.let { shared = it }
      runCatching { engine.photoLibrary() }.getOrNull()?.let { library = it }
    }
  }
  LaunchedEffect(key) { if (key != "hidden") hiddenOpen = false; selected = emptySet() }

  fun go(to: String) {
    collections = false
    if (to == "hidden") unlock(context) { hiddenOpen = true; key = "hidden" } else key = to
  }

  /**
   * A flag on photos: recorded and shown at once (the index is read again), then sent by a
   * Photos sync, which sends what waits before it reads. Asked for once the Undo has had its
   * moment: a change undone before it went is taken off the queue, and Apple hears nothing.
   */
  fun flag(ids: List<String>, f: PFlag, on: Boolean, undo: String? = null) = scope.launch {
    selected = emptySet()
    val result = runCatching { withContext(Dispatchers.IO) { engine.photosSetFlag(ids, f, on) } }
    generation++
    result.exceptionOrNull()?.let { snackbar.showSnackbar((it as? EngineException.Failed)?.reason ?: "Photos couldn't be changed.") }
    if (result.isSuccess && undo != null) {
      val back = snackbar.showSnackbar(undo, actionLabel = "Undo", duration = SnackbarDuration.Short)
      if (back == SnackbarResult.ActionPerformed) {
        runCatching { withContext(Dispatchers.IO) { engine.photosSetFlag(ids, f, !on) } }
        generation++
      }
    }
    if (result.isSuccess) syncPhotos(context)
  }

  BackHandler(enabled = collections || selected.isNotEmpty() || key != "library") {
    when {
      selected.isNotEmpty() -> selected = emptySet()
      collections -> collections = false
      else -> key = "library"
    }
  }

  val idx = index
  open?.let { at ->
    if (idx != null && at in idx.ids.indices) {
      Viewer(engine, idx, at, readOnly, onClose = { open = null }, onFlag = { id, f, on, undo -> flag(listOf(id), f, on, undo) })
      return
    }
  }
  if (collections) {
    Collections(
      engine, albums, shared, library, onBack = { collections = false }, onOpen = ::go,
      onLibrary = { which ->
        scope.launch {
          withContext(Dispatchers.IO) { runCatching { engine.photoSetLibrary(which) } }
          generation++
        }
      },
    )
    return
  }

  Box(Modifier.fillMaxSize()) {
    Column(Modifier.fillMaxSize()) {
      Row(Modifier.padding(start = 4.dp, end = 4.dp, top = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        if (selected.isNotEmpty()) {
          IconButton(onClick = { selected = emptySet() }) { Icon(painterResource(R.drawable.ic_close), contentDescription = "Cancel selection") }
          Text("${selected.size} selected", style = MaterialTheme.typography.titleMedium)
        } else if (key != "library") {
          IconButton(onClick = { key = "library" }) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Library") }
        }
        Spacer(Modifier.weight(1f))
        if (selected.isEmpty()) {
          IconButton(onClick = { collections = true }) {
            Icon(painterResource(R.drawable.ic_albums), contentDescription = "Collections", tint = mark(Palette.App.Photos))
          }
        }
      }
      if (selected.isEmpty()) Column(Modifier.padding(horizontal = 20.dp).padding(bottom = 8.dp)) {
        Text(place.title, style = MaterialTheme.typography.headlineLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (idx != null) {
          val n = idx.ids.size
          Text(
            listOfNotNull(span(idx), if (n == 1) "1 item" else "$n items").joinToString(" · "),
            style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
          )
        }
      }
      when {
        idx == null -> {}
        idx.ids.isEmpty() -> Text(
          when (key) { "deleted" -> "Nothing deleted in the last 30 days"; "favorites" -> "No favorites yet"; else -> "No photos" },
          style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(24.dp),
        )
        else -> Grid(
          engine, idx, columns, selected,
          onColumns = { columns = it },
          onTap = { i ->
            val id = idx.ids[i]
            if (selected.isNotEmpty()) selected = if (id in selected) selected - id else selected + id else open = i
          },
          onHold = { i -> selected = selected + idx.ids[i] },
          modifier = Modifier.weight(1f),
        )
      }
      if (selected.isNotEmpty()) SelectionBar(
        key, selected.size, readOnly,
        onShare = { val ids = selected.toList(); selected = emptySet(); scope.launch { share(context, engine, ids) } },
        onFavorite = { flag(selected.toList(), PFlag.FAVORITE, true) },
        onHide = { flag(selected.toList(), PFlag.HIDDEN, key != "hidden", if (key != "hidden") "Hidden" else "Shown again") },
        onDelete = {
          if (key == "deleted") flag(selected.toList(), PFlag.EXPUNGED, true)
          else flag(selected.toList(), PFlag.DELETED, true, "Moved to Recently Deleted")
        },
        onRecover = { flag(selected.toList(), PFlag.DELETED, false) },
      )
    }
    SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).padding(bottom = if (selected.isNotEmpty()) 72.dp else 8.dp))
  }
}

/**
 * The grid: months headed, photos square and edge to edge with a hairline between, as the
 * desktop draws them. Two fingers change how many a row holds (3 to 8); a long library gets a
 * scrubber on the right edge that names the month under the thumb.
 */
@Composable
private fun Grid(
  engine: AccountInterface, idx: PIndex, columns: Int, selected: Set<String>,
  onColumns: (Int) -> Unit, onTap: (Int) -> Unit, onHold: (Int) -> Unit, modifier: Modifier,
) {
  val rows = remember(idx) { cells(idx) }
  val months = remember(rows) { rows.withIndex().filter { it.value is Cell.Month }.map { it.index to (it.value as Cell.Month).label } }
  val state = rememberLazyGridState()
  val scope = rememberCoroutineScope()
  var scrubbing by remember { mutableStateOf<String?>(null) }
  var scrubY by remember { mutableFloatStateOf(0f) }
  val density = LocalDensity.current
  BoxWithConstraints(modifier.fillMaxWidth()) {
    val px = with(density) { (maxWidth / columns).roundToPx() }
    LazyVerticalGrid(
      GridCells.Fixed(columns), state = state,
      horizontalArrangement = Arrangement.spacedBy(2.dp), verticalArrangement = Arrangement.spacedBy(2.dp),
      contentPadding = PaddingValues(bottom = 16.dp),
      modifier = Modifier.fillMaxSize().pointerInput(columns) {
        // Two fingers only: one finger still scrolls the grid.
        awaitEachGesture {
          awaitFirstDown(requireUnconsumed = false)
          var zoom = 1f
          while (true) {
            val event = awaitPointerEvent(PointerEventPass.Initial)
            if (event.changes.none { it.pressed }) break
            if (event.changes.count { it.pressed } >= 2) {
              zoom *= event.calculateZoom()
              event.changes.forEach { it.consume() }
            }
          }
          val next = when {
            zoom > 1.25f -> (columns - if (columns > 5) 2 else 1).coerceAtLeast(3)
            zoom < 0.8f -> (columns + if (columns >= 5) 2 else 1).coerceAtMost(8)
            else -> columns
          }
          if (next != columns) onColumns(next)
        }
      },
    ) {
      items(
        rows.size,
        key = { i -> when (val c = rows[i]) { is Cell.Month -> "m${c.first}"; is Cell.Photo -> idx.ids[c.at] } },
        span = { i -> if (rows[i] is Cell.Month) GridItemSpan(maxLineSpan) else GridItemSpan(1) },
        contentType = { i -> if (rows[i] is Cell.Month) 0 else 1 },
      ) { i ->
        when (val c = rows[i]) {
          is Cell.Month -> Text(
            c.label, style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(start = 16.dp, top = if (c.first == 0) 4.dp else 20.dp, bottom = 8.dp),
          )
          is Cell.Photo -> {
            val id = idx.ids[c.at]
            val m = idx.marks[c.at].toInt()
            Thumb(engine, id, px, video = (m and 3) == 1, favorite = (m and 4) != 0, picked = id in selected, onTap = { onTap(c.at) }, onHold = { onHold(c.at) })
          }
        }
      }
    }
    if (idx.ids.size > 300) {
      // The scrubber: drag the right edge to move through the years; the month shows beside it.
      Box(
        Modifier.align(Alignment.CenterEnd).width(36.dp).fillMaxHeight()
          .semantics { contentDescription = "Scroll by date" }
          .pointerInput(rows) {
            detectVerticalDragGestures(
              onDragStart = { scrubY = it.y },
              onDragEnd = { scrubbing = null },
              onDragCancel = { scrubbing = null },
            ) { change, _ ->
              scrubY = change.position.y.coerceIn(0f, size.height.toFloat())
              val target = ((scrubY / size.height) * (rows.size - 1)).roundToInt()
              scrubbing = months.lastOrNull { it.first <= target }?.second
              scope.launch { state.scrollToItem(target) }
            }
          },
      )
      scrubbing?.let { label ->
        Surface(
          color = MaterialTheme.colorScheme.inverseSurface, contentColor = MaterialTheme.colorScheme.inverseOnSurface,
          shape = MaterialTheme.shapes.medium,
          modifier = Modifier.align(Alignment.TopEnd).offset { IntOffset(-with(density) { 48.dp.roundToPx() }, scrubY.roundToInt() - 40) },
        ) { Text(label, style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) }
      }
    }
  }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Thumb(engine: AccountInterface, id: String, px: Int, video: Boolean, favorite: Boolean, picked: Boolean, onTap: () -> Unit, onHold: () -> Unit) {
  val context = LocalContext.current
  val picture by produceState(thumbs.get(id), id) { if (value == null) value = thumbnail(context, engine, id, px) }
  Box(
    Modifier.aspectRatio(1f).background(MaterialTheme.colorScheme.surfaceContainerHigh)
      .combinedClickable(onClick = onTap, onLongClick = onHold, onLongClickLabel = "Select"),
  ) {
    picture?.let { Image(it, contentDescription = if (video) "Video" else "Photo", contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize()) }
    if (favorite) Icon(painterResource(R.drawable.ic_heart_fill), contentDescription = "Favorite", tint = Color.White, modifier = Modifier.align(Alignment.BottomStart).padding(5.dp).size(14.dp))
    if (video) Icon(painterResource(R.drawable.ic_play), contentDescription = null, tint = Color.White, modifier = Modifier.align(Alignment.BottomEnd).padding(5.dp).size(14.dp))
    if (picked) {
      Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.25f)))
      Box(
        Modifier.align(Alignment.BottomEnd).padding(6.dp).size(22.dp).background(MaterialTheme.colorScheme.primary, CircleShape).border(1.5.dp, Color.White, CircleShape),
        contentAlignment = Alignment.Center,
      ) { Icon(painterResource(R.drawable.ic_check), contentDescription = "Selected", tint = MaterialTheme.colorScheme.onPrimary, modifier = Modifier.size(14.dp)) }
    }
  }
}

/** What a selection can do, at the bottom: Share, Favorite, Hide (Show), Delete; Recover in Recently Deleted. */
@Composable
private fun SelectionBar(key: String, n: Int, readOnly: Boolean, onShare: () -> Unit, onFavorite: () -> Unit, onHide: () -> Unit, onDelete: () -> Unit, onRecover: () -> Unit) {
  Surface(color = MaterialTheme.colorScheme.surfaceContainer) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp), horizontalArrangement = Arrangement.SpaceEvenly) {
      if (readOnly) {
        // A shared album's photos are shared from here, never changed.
        IconButton(onClick = onShare) { Icon(painterResource(R.drawable.ic_share), contentDescription = "Share") }
      } else if (key == "deleted") {
        TextButton(onClick = onRecover) { Text("Recover") }
        TextButton(onClick = onDelete) { Text(if (n == 1) "Delete Now" else "Delete $n Now", color = MaterialTheme.colorScheme.error) }
      } else {
        IconButton(onClick = onShare) { Icon(painterResource(R.drawable.ic_share), contentDescription = "Share") }
        IconButton(onClick = onFavorite) { Icon(painterResource(R.drawable.ic_heart), contentDescription = "Favorite") }
        IconButton(onClick = onHide) { Icon(painterResource(R.drawable.ic_eye_off), contentDescription = if (key == "hidden") "Show" else "Hide") }
        IconButton(onClick = onDelete) { Icon(painterResource(R.drawable.ic_trash), contentDescription = "Delete", tint = MaterialTheme.colorScheme.error) }
      }
    }
  }
}

/**
 * Collections: the places the desktop's sidebar lists, then the albums and the shared albums, each
 * with its newest photo. With a Shared Library, which library the views show is chosen here, as
 * the desktop's switcher chooses it.
 */
@Composable
private fun Collections(
  engine: AccountInterface, albums: List<PAlbum>, shared: List<PSharedAlbum>, library: PLibraryChoice?,
  onBack: () -> Unit, onOpen: (String) -> Unit, onLibrary: (PLibrary) -> Unit,
) {
  BackHandler(onBack = onBack)
  val pairs = albums.chunked(2)
  val sharedPairs = shared.chunked(2)
  LazyColumn(contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 24.dp)) {
    item {
      IconButton(onClick = onBack, modifier = Modifier.offset(x = (-12).dp)) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Library") }
      Text("Collections", style = MaterialTheme.typography.headlineLarge, modifier = Modifier.padding(start = 4.dp, bottom = 12.dp))
    }
    if (library?.hasShared == true) item {
      SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(bottom = 16.dp)) {
        LIBRARIES.forEachIndexed { i, (which, label) ->
          SegmentedButton(
            selected = library.showing == which, onClick = { onLibrary(which) },
            shape = SegmentedButtonDefaults.itemShape(i, LIBRARIES.size),
          ) { Text(label, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        }
      }
    }
    item {
      GroupCard(PLACES.drop(1), inset = 56) { p ->
        Row(Modifier.fillMaxWidth().clickable { onOpen(p.key) }.padding(horizontal = 16.dp, vertical = 14.dp), verticalAlignment = Alignment.CenterVertically) {
          Icon(painterResource(p.icon), contentDescription = null, tint = mark(Palette.App.Photos), modifier = Modifier.size(22.dp))
          Spacer(Modifier.width(18.dp))
          Text(p.title, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
          if (p.key == "hidden") Icon(painterResource(R.drawable.ic_lock), contentDescription = "Locked", tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(16.dp))
          Icon(painterResource(R.drawable.ic_chevron_right), contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(18.dp))
        }
      }
    }
    if (albums.isNotEmpty()) {
      item { SectionLabel("Albums") }
      items(pairs.size) { r ->
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.padding(bottom = 12.dp)) {
          for (a in pairs[r]) AlbumTile(engine, a, Modifier.weight(1f)) { onOpen("album:${a.id}") }
          if (pairs[r].size == 1) Spacer(Modifier.weight(1f))
        }
      }
    }
    if (shared.isNotEmpty()) {
      item { SectionLabel("Shared Albums") }
      items(sharedPairs.size) { r ->
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), modifier = Modifier.padding(bottom = 12.dp)) {
          for (a in sharedPairs[r]) SharedAlbumTile(engine, a, Modifier.weight(1f)) { onOpen("$SHARED${a.zone}") }
          if (sharedPairs[r].size == 1) Spacer(Modifier.weight(1f))
        }
      }
    }
  }
}

/** A shared album: its newest photo, its name and how many it holds, as icloud.com lists them. */
@Composable
private fun SharedAlbumTile(engine: AccountInterface, a: PSharedAlbum, modifier: Modifier, onClick: () -> Unit) {
  val context = LocalContext.current
  val cover by produceState<ImageBitmap?>(null, a.zone, a.cover) { value = a.cover?.let { thumbnail(context, engine, it, 360) } }
  Column(modifier.clickable(onClick = onClick).semantics(mergeDescendants = true) {}) {
    Box(Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(12.dp)).background(MaterialTheme.colorScheme.surfaceContainerHigh)) {
      cover?.let { Image(it, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize()) }
    }
    Text(a.title ?: "Shared Album", style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 6.dp))
    Text(
      if (a.count == 1u) "1 item" else "${a.count} items",
      style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
  }
}

@Composable
private fun AlbumTile(engine: AccountInterface, a: PAlbum, modifier: Modifier, onClick: () -> Unit) {
  val context = LocalContext.current
  val cover by produceState<Pair<ImageBitmap?, Int>>(null to -1, a.id) {
    val idx = withContext(Dispatchers.IO) { runCatching { engine.photoIndex(PView.Album(a.id)) }.getOrNull() }
    value = (idx?.ids?.firstOrNull()?.let { thumbnail(context, engine, it, 360) }) to (idx?.ids?.size ?: 0)
  }
  Column(modifier.clickable(onClick = onClick).semantics(mergeDescendants = true) {}) {
    Box(Modifier.fillMaxWidth().aspectRatio(1f).clip(RoundedCornerShape(12.dp)).background(MaterialTheme.colorScheme.surfaceContainerHigh)) {
      cover.first?.let { Image(it, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize()) }
    }
    Text(a.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 6.dp))
    if (cover.second >= 0) Text("${cover.second}", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
  }
}

/**
 * A photo full screen: swipe for the next, pinch or double-tap to zoom, tap to show or hide the
 * bars; Share, Favorite and Delete (with Undo) at the bottom; a film plays where it is.
 */
@Composable
private fun Viewer(engine: AccountInterface, idx: PIndex, start: Int, readOnly: Boolean, onClose: () -> Unit, onFlag: (String, PFlag, Boolean, String?) -> Unit) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  BackHandler(onBack = onClose)
  FullScreen()
  val pager = rememberPagerState(initialPage = start) { idx.ids.size }
  var bars by remember { mutableStateOf(true) }
  Box(Modifier.fillMaxSize().background(Color.Black)) {
    HorizontalPager(pager, key = { idx.ids[it] }, beyondViewportPageCount = 1) { page ->
      Page(engine, idx.ids[page], video = (idx.marks[page].toInt() and 3) == 1, current = pager.currentPage == page) { bars = !bars }
    }
    val at = pager.currentPage
    val id = idx.ids[at]
    val favorite = (idx.marks[at].toInt() and 4) != 0
    AnimatedVisibility(bars, enter = fadeIn(), exit = fadeOut(), modifier = Modifier.align(Alignment.TopStart)) {
      Row(Modifier.fillMaxWidth().background(Color.Black.copy(alpha = 0.35f)).statusBarsPadding().padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
        IconButton(onClick = onClose) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Back", tint = Color.White) }
        Text(
          Instant.ofEpochSecond(idx.taken[at]).atZone(ZoneId.systemDefault()).format(DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM, FormatStyle.SHORT)),
          style = MaterialTheme.typography.titleSmall, color = Color.White,
        )
      }
    }
    AnimatedVisibility(bars, enter = fadeIn(), exit = fadeOut(), modifier = Modifier.align(Alignment.BottomCenter)) {
      Row(
        Modifier.fillMaxWidth().background(Color.Black.copy(alpha = 0.35f)).navigationBarsPadding().padding(horizontal = 24.dp, vertical = 6.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
      ) {
        IconButton(onClick = { scope.launch { share(context, engine, listOf(id)) } }) { Icon(painterResource(R.drawable.ic_share), contentDescription = "Share", tint = Color.White) }
        // A shared album's photo is looked at and shared, never changed from here.
        if (!readOnly) {
          IconButton(onClick = { onFlag(id, PFlag.FAVORITE, !favorite, null) }) {
            Icon(painterResource(if (favorite) R.drawable.ic_heart_fill else R.drawable.ic_heart), contentDescription = if (favorite) "Unfavorite" else "Favorite", tint = Color.White)
          }
          IconButton(onClick = { onFlag(id, PFlag.DELETED, true, "Moved to Recently Deleted"); if (idx.ids.size <= 1) onClose() }) {
            Icon(painterResource(R.drawable.ic_trash), contentDescription = "Delete", tint = Color.White)
          }
        }
      }
    }
  }
}

@Composable
private fun Page(engine: AccountInterface, id: String, video: Boolean, current: Boolean, onTap: () -> Unit) {
  val context = LocalContext.current
  val picture by produceState<ImageBitmap?>(thumbs.get(id), id) {
    val big = withContext(Dispatchers.IO) {
      val file = cached(context, id, "medium")
      runCatching { if (file.exists() || engine.photoFile(id, PSize.MEDIUM, file.path)) decode(file, 2048) else null }.getOrNull()
    }
    if (big != null) value = big
  }
  var film by remember { mutableStateOf<File?>(null) }
  var playing by remember { mutableStateOf(false) }
  LaunchedEffect(current) { if (!current) playing = false }
  LaunchedEffect(playing) {
    if (playing && film == null) film = withContext(Dispatchers.IO) {
      val file = cached(context, id, "video")
      runCatching { if (file.exists() || engine.photoFile(id, PSize.VIDEO, file.path)) file else null }.getOrNull()
    }
  }
  var scale by remember { mutableFloatStateOf(1f) }
  var offset by remember { mutableStateOf(Offset.Zero) }
  LaunchedEffect(current) { if (!current) { scale = 1f; offset = Offset.Zero } }
  Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
    val f = film
    if (playing && f != null) {
      AndroidView({ c -> VideoView(c).apply { setVideoPath(f.path); setOnPreparedListener { it.isLooping = false; start() } } }, Modifier.fillMaxSize())
    } else {
      picture?.let {
        Image(
          it, contentDescription = if (video) "Video" else "Photo", contentScale = ContentScale.Fit,
          modifier = Modifier.fillMaxSize()
            .pointerInput(id) {
              detectTapGestures(
                onTap = { onTap() },
                onDoubleTap = { if (scale > 1f) { scale = 1f; offset = Offset.Zero } else scale = 2.5f },
              )
            }
            .pointerInput(id, scale > 1f) {
              // Pinch zooms; once zoomed a drag pans. At 1x a swipe still turns the page.
              if (scale > 1f) detectTransformGestures { _, pan, zoom, _ ->
                scale = (scale * zoom).coerceIn(1f, 5f)
                offset = if (scale > 1f) offset + pan else Offset.Zero
              } else awaitEachGesture {
                awaitFirstDown(requireUnconsumed = false)
                var zoom = 1f
                while (true) {
                  val event = awaitPointerEvent()
                  if (event.changes.none { it.pressed }) break
                  if (event.changes.count { it.pressed } >= 2) {
                    zoom *= event.calculateZoom()
                    scale = zoom.coerceIn(1f, 5f)
                    event.changes.forEach { it.consume() }
                  }
                }
              }
            }
            .graphicsLayer { scaleX = scale; scaleY = scale; translationX = offset.x; translationY = offset.y },
        )
      }
      if (video) IconButton(onClick = { playing = true }, modifier = Modifier.size(72.dp).background(Color.Black.copy(alpha = 0.4f), CircleShape)) {
        Icon(painterResource(R.drawable.ic_play), contentDescription = "Play", tint = Color.White, modifier = Modifier.size(36.dp))
      }
    }
  }
}

package io.github.yashwanthkamireddi.skykeep.ui

import androidx.compose.runtime.saveable.rememberSaveable
import android.content.ContentResolver
import android.graphics.BitmapFactory
import android.os.Bundle
import android.util.LruCache
import android.widget.VideoView
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.sync.Outcome
import io.github.yashwanthkamireddi.skykeep.sync.PHOTOS_AUTHORITY
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.PAlbum
import uniffi.quince_mobile.PIndex
import uniffi.quince_mobile.PSize
import uniffi.quince_mobile.PView

/** Thumbnails decoded, the most recently shown kept: a grid scrolls back without decoding again. */
private val thumbs = object : LruCache<String, ImageBitmap>(48 * 1024 * 1024) {
  override fun sizeOf(key: String, value: ImageBitmap) = value.width * value.height * 4
}

/** A few pictures fetched at once: a grid flung fast asks for hundreds. */
private val fetching = Semaphore(6)

private fun syncPhotos(context: android.content.Context) {
  phoneAccount(context)?.let {
    ContentResolver.requestSync(it, PHOTOS_AUTHORITY, Bundle().apply {
      putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
      putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
    })
  }
}

/** Where a photo's picture of a size is kept on the phone. */
private fun cached(context: android.content.Context, id: String, size: String): File =
  File(context.cacheDir, "photos/$size/${id.hashCode().toUInt().toString(16)}-${id.length}")

/** A picture decoded no larger than `long` pixels on its long side. */
private fun decode(file: File, long: Int): ImageBitmap? {
  val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
  BitmapFactory.decodeFile(file.path, bounds)
  if (bounds.outWidth <= 0) return null
  var sample = 1
  while (maxOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= long) sample *= 2
  return BitmapFactory.decodeFile(file.path, BitmapFactory.Options().apply { inSampleSize = sample })?.asImageBitmap()
}

/**
 * Photos: the library as a grid, newest first, read as one compact index (a library of any size
 * opens at once); each thumbnail fetched once as it comes into view and kept; a photo opened full
 * screen, a film played. Library, Favorites, Videos, Recents and the albums along the top.
 */
@Composable
fun PhotosPage(engine: Account) {
  val context = LocalContext.current
  var view by remember { mutableStateOf<PView>(PView.Library) }
  var albums by remember { mutableStateOf<List<PAlbum>>(emptyList()) }
  var index by remember { mutableStateOf<PIndex?>(null) }
  var open by rememberSaveable { mutableStateOf<String?>(null) }
  var generation by remember { mutableIntStateOf(0) }

  LaunchedEffect(Unit) { syncPhotos(context) }
  DisposableEffect(Unit) {
    val stop = Outcome.watch(context) { generation++ }
    onDispose { stop() }
  }
  LaunchedEffect(view, generation) {
    withContext(Dispatchers.IO) {
      runCatching { engine.photoIndex(view) }.getOrNull()?.let { index = it }
      runCatching { engine.photoAlbums() }.getOrNull()?.let { albums = it.filter { a -> !a.folder } }
    }
  }

  open?.let { id ->
    Viewer(engine, id) { open = null }
    return
  }

  Column(Modifier.fillMaxSize()) {
    Text("Photos", style = MaterialTheme.typography.headlineMedium, modifier = Modifier.padding(start = 20.dp, top = 16.dp))
    Row(Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = 16.dp, vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
      val fixed = listOf("Library" to PView.Library, "Favorites" to PView.Favorites, "Videos" to PView.Videos, "Recents" to PView.Recents)
      for ((label, v) in fixed) FilterChip(selected = view == v, onClick = { view = v }, label = { Text(label) })
      for (a in albums) {
        val v = PView.Album(a.id)
        FilterChip(selected = view == v, onClick = { view = v }, label = { Text(a.name) })
      }
    }
    val idx = index
    when {
      idx == null -> {}
      idx.ids.isEmpty() -> Text("No photos", color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(24.dp))
      else -> LazyVerticalGrid(GridCells.Adaptive(96.dp), horizontalArrangement = Arrangement.spacedBy(2.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        itemsIndexed(idx.ids, key = { _, id -> id }) { i, id ->
          val video = (idx.marks[i].toInt() and 3) == 1
          Thumb(engine, id, video) { open = id }
        }
      }
    }
  }
}

@Composable
private fun Thumb(engine: Account, id: String, video: Boolean, onClick: () -> Unit) {
  val context = LocalContext.current
  val picture by produceState(thumbs.get(id), id) {
    if (value != null) return@produceState
    value = withContext(Dispatchers.IO) {
      fetching.withPermit {
        val file = cached(context, id, "thumb")
        runCatching { if (engine.photoFile(id, PSize.THUMB, file.path)) decode(file, 360) else null }.getOrNull()
      }
    }?.also { thumbs.put(id, it) }
  }
  Box(Modifier.aspectRatio(1f).background(MaterialTheme.colorScheme.surfaceContainerHigh).clickable(onClick = onClick)) {
    picture?.let { Image(it, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize()) }
    if (video) Text("▶", color = Color.White, modifier = Modifier.align(Alignment.BottomStart).padding(6.dp))
  }
}

/** A photo full screen at the viewer's size; a film fetched and played. */
@Composable
private fun Viewer(engine: Account, id: String, onClose: () -> Unit) {
  val context = LocalContext.current
  BackHandler(onBack = onClose)
  val item by produceState<uniffi.quince_mobile.PItem?>(null, id) { value = withContext(Dispatchers.IO) { runCatching { engine.photoItem(id) }.getOrNull() } }
  val picture by produceState<ImageBitmap?>(thumbs.get(id), id) {
    val big = withContext(Dispatchers.IO) {
      val file = cached(context, id, "medium")
      runCatching { if (engine.photoFile(id, PSize.MEDIUM, file.path)) decode(file, 2048) else null }.getOrNull()
    }
    if (big != null) value = big
  }
  var film by remember { mutableStateOf<File?>(null) }
  LaunchedEffect(item) {
    if (item?.video == true) {
      film = withContext(Dispatchers.IO) {
        val file = cached(context, id, "video")
        runCatching { if (engine.photoFile(id, PSize.VIDEO, file.path)) file else null }.getOrNull()
      }
    }
  }
  Box(Modifier.fillMaxSize().background(Color.Black)) {
    val f = film
    if (f != null) {
      AndroidView({ c -> VideoView(c).apply { setVideoPath(f.path); setOnPreparedListener { it.isLooping = false; start() } } }, Modifier.fillMaxSize())
    } else {
      picture?.let { Image(it, contentDescription = item?.filename, contentScale = ContentScale.Fit, modifier = Modifier.fillMaxSize()) }
    }
    IconButton(onClick = onClose, modifier = Modifier.padding(8.dp)) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Back", tint = Color.White) }
  }
}

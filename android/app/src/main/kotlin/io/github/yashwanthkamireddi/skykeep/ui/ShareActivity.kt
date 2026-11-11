package io.github.yashwanthkamireddi.skykeep.ui

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.unit.dp
import io.github.yashwanthkamireddi.skykeep.Engine
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.sync.SendToIphone
import io.github.yashwanthkamireddi.skykeep.ui.theme.SkykeepTheme
import java.io.File
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Send to iPhone from any app's Share sheet: a sheet naming what was shared and where it will
 * appear, and nothing sent until the person taps Send (any app can open this screen, so none
 * can push files into someone's iCloud on its own). Then the files are copied while this app
 * may read them and handed to [SendToIphone]. Signed out, it says so instead; nothing is sent.
 */
class ShareActivity : ComponentActivity() {
  /** What was shared: one item or several. */
  private fun shared(intent: Intent): List<Uri> = when (intent.action) {
    Intent.ACTION_SEND -> listOfNotNull(
      if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
      else @Suppress("DEPRECATION") intent.getParcelableExtra(Intent.EXTRA_STREAM),
    )
    Intent.ACTION_SEND_MULTIPLE ->
      (if (Build.VERSION.SDK_INT >= 33) intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
      else @Suppress("DEPRECATION") intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)).orEmpty()
    else -> emptyList()
  }

  /**
   * What a shared item is called: its own name when the sharing app gives one, else the last
   * part of its address; with the extension its type has when the name lacks one (a photo
   * shared nameless as "image/jpeg" is a .jpg, so it goes to Photos as a photo, not to Drive).
   */
  private fun nameOf(uri: Uri): String {
    val given = runCatching {
      contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)
        ?.use { c -> if (c.moveToFirst()) c.getString(0) else null }
    }.getOrNull() ?: uri.lastPathSegment?.substringAfterLast('/')
    val name = given?.let { io.github.yashwanthkamireddi.skykeep.sync.DriveFiles.safeName(it) }?.takeIf { it != "file" } ?: "Shared file"
    if ('.' in name.substringAfterLast('/').drop(1)) return name
    val type = contentResolver.getType(uri) ?: intent.type
    val ext = type?.let { android.webkit.MimeTypeMap.getSingleton().getExtensionFromMimeType(it) }
    return if (ext != null) "$name.$ext" else name
  }

  /**
   * A shared item copied under its own name into this share's batch, in a folder of its own
   * numbered in the order shared (two items of one name never clash), where it waits until it
   * has gone (SendToIphone). Only content addresses are taken, as Android has apps share: a
   * plain file path could point into Skykeep's own storage and have it upload its private files.
   */
  private fun copy(batch: File, n: Int, uri: Uri): File? = runCatching {
    if (uri.scheme != android.content.ContentResolver.SCHEME_CONTENT) return null
    val file = File(batch, "$n/${nameOf(uri)}").also { it.parentFile?.mkdirs() }
    contentResolver.openInputStream(uri)!!.use { input -> file.outputStream().use { input.copyTo(it) } }
    file
  }.getOrNull()

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val uris = shared(intent).filter { it.scheme == android.content.ContentResolver.SCHEME_CONTENT }
    setContent {
      SkykeepTheme {
        var stage by remember { mutableStateOf("checking") }
        var n by remember { mutableStateOf(0) }
        var names by remember { mutableStateOf<List<String>>(emptyList()) }
        var go by remember { mutableStateOf(false) }
        LaunchedEffect(Unit) {
          stage = withContext(Dispatchers.IO) {
            if (!Engine.account(applicationContext).hasSession()) return@withContext "signedOut"
            if (uris.isEmpty()) return@withContext "nothing"
            names = uris.take(3).map { nameOf(it) }
            "ask"
          }
        }
        LaunchedEffect(go) {
          if (!go) return@LaunchedEffect
          stage = "copying"
          stage = withContext(Dispatchers.IO) {
            val tag = Engine.account(applicationContext).accountTag() ?: return@withContext "signedOut"
            val batch = File(SendToIphone.batches(applicationContext), UUID.randomUUID().toString())
            val files = uris.mapIndexedNotNull { i, uri -> copy(batch, i, uri) }
            n = files.size
            if (files.isEmpty()) { batch.deleteRecursively(); return@withContext "nothing" }
            SendToIphone.start(applicationContext, batch, tag)
            "sending"
          }
        }
        // A scrim over the sharing app; the sheet at the bottom.
        Box(
          Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.32f))
            .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) { finish() },
          contentAlignment = Alignment.BottomCenter,
        ) {
          Surface(
            shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp),
            color = MaterialTheme.colorScheme.surfaceContainer,
            modifier = Modifier.fillMaxWidth().clickable(interactionSource = remember { MutableInteractionSource() }, indication = null) {},
          ) {
            Column(Modifier.navigationBarsPadding().padding(24.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
              Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
                Image(painterResource(R.drawable.app_drive), contentDescription = null, modifier = Modifier.size(44.dp))
                Text("Send to iPhone", style = MaterialTheme.typography.titleLarge)
              }
              when (stage) {
                "ask" -> {
                  val more = uris.size - names.size
                  Text(
                    names.joinToString(", ") + (if (more > 0) " and $more more" else ""),
                    style = MaterialTheme.typography.bodyLarge, maxLines = 3,
                    overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                  )
                  Text(
                    "Photos and videos go to your iPhone's Photos; other files to Files, in iCloud Drive › “${SendToIphone.folderName()}”.",
                    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                  )
                  Row(Modifier.align(Alignment.End), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    androidx.compose.material3.TextButton(onClick = { finish() }) { Text("Cancel") }
                    Button(onClick = { go = true }) { Text(if (uris.size == 1) "Send" else "Send ${uris.size} Items") }
                  }
                }
                "checking", "copying" -> Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
                  CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
                  Text("Getting ready", style = MaterialTheme.typography.bodyLarge)
                }
                "sending" -> Text(
                  "Sending ${if (n == 1) "1 item" else "$n items"}. Photos and videos appear in your iPhone's Photos; other files in Files, in iCloud Drive › “${SendToIphone.folderName()}”. You can close this: a notification says when it's done.",
                  style = MaterialTheme.typography.bodyLarge,
                )
                "signedOut" -> Text("Open Skykeep and sign in first, then share again.", style = MaterialTheme.typography.bodyLarge)
                else -> Text("Nothing here could be read to send.", style = MaterialTheme.typography.bodyLarge)
              }
              if (stage !in setOf("checking", "ask", "copying")) Button(onClick = { finish() }, modifier = Modifier.align(Alignment.End)) { Text("Done") }
            }
          }
        }
      }
    }
  }
}

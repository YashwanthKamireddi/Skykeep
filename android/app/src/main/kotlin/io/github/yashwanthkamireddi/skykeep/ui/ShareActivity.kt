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
 * Send to iPhone from any app's Share sheet: the shared files copied while this app may read
 * them, handed to [SendToIphone], and a sheet saying where they will appear. Signed out, it
 * says so instead; nothing is sent.
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
    val name = given?.replace('/', '_')?.takeIf { it.isNotBlank() } ?: "Shared file"
    if ('.' in name.substringAfterLast('/').drop(1)) return name
    val type = contentResolver.getType(uri) ?: intent.type
    val ext = type?.let { android.webkit.MimeTypeMap.getSingleton().getExtensionFromMimeType(it) }
    return if (ext != null) "$name.$ext" else name
  }

  /** A shared item copied under its own name, in a folder of its own (two shares never clash). */
  private fun copy(uri: Uri): File? = runCatching {
    val file = File(cacheDir, "send/${UUID.randomUUID()}/${nameOf(uri)}").also { it.parentFile?.mkdirs() }
    contentResolver.openInputStream(uri)!!.use { input -> file.outputStream().use { input.copyTo(it) } }
    file
  }.getOrNull()

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    val uris = shared(intent)
    setContent {
      SkykeepTheme {
        var stage by remember { mutableStateOf("copying") }
        var n by remember { mutableStateOf(0) }
        LaunchedEffect(Unit) {
          stage = withContext(Dispatchers.IO) {
            if (!Engine.account(applicationContext).hasSession()) return@withContext "signedOut"
            val files = uris.mapNotNull { copy(it) }
            n = files.size
            if (files.isEmpty()) return@withContext "nothing"
            SendToIphone.start(applicationContext, files)
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
                "copying" -> Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
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
              if (stage != "copying") Button(onClick = { finish() }, modifier = Modifier.align(Alignment.End)) { Text("Done") }
            }
          }
        }
      }
    }
  }
}

package io.github.yashwanthkamireddi.skykeep.ui

import android.accounts.Account
import android.graphics.BitmapFactory
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.yashwanthkamireddi.skykeep.sync.ContactsSync
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.AccountInterface

/** The account as shown: its name, its address, and its own card's picture, decoded small. */
private data class Profile(val name: String?, val email: String?, val picture: ImageBitmap?)

/**
 * The signed-in account, as the desktop's sidebar shows it: the person's own card's picture
 * (the card Contacts marks as theirs, read from this phone's Contacts, never fetched), else the
 * first letter of their name; their name, else iCloud; and their address, else the Apple ID.
 * `key` reads it again (after a sync has brought the card).
 */
@Composable
fun AccountRow(
  engine: AccountInterface, account: Account?, key: Any? = null, size: Int = 44,
  trailing: @Composable RowScope.() -> Unit = {},
) {
  val context = LocalContext.current
  val id = androidx.compose.runtime.remember { runCatching { engine.appleId() }.getOrNull() }
  val profile by produceState<Profile?>(null, account, key) {
    value = withContext(Dispatchers.IO) {
      runCatching {
        val p = engine.accountProfile()
        val px = (size * context.resources.displayMetrics.density).toInt().coerceAtLeast(1)
        val picture = p.meCard?.let { guid -> account?.let { ContactsSync.pictureOf(context.contentResolver, it, guid) } }
          ?.let { bytes ->
            val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
            // Decoded no larger than shown: a 720-pixel face for a 44-point circle.
            var sample = 1
            while (minOf(bounds.outWidth, bounds.outHeight) / (sample * 2) >= px) sample *= 2
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, BitmapFactory.Options().apply { inSampleSize = sample })
          }?.asImageBitmap()
        Profile(p.name?.takeIf { it.isNotBlank() }, p.email?.takeIf { it.isNotBlank() }, picture)
      }.getOrNull()
    }
  }
  val name = profile?.name
  val detail = profile?.email ?: id
  Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
    val picture = profile?.picture
    if (picture != null) {
      Image(
        picture, contentDescription = null, contentScale = ContentScale.Crop,
        modifier = Modifier.size(size.dp).clip(CircleShape),
      )
    } else {
      Box(
        Modifier.size(size.dp).background(MaterialTheme.colorScheme.primaryContainer, CircleShape),
        contentAlignment = Alignment.Center,
      ) {
        Text(
          ((name ?: id)?.firstOrNull() ?: 'i').uppercase(),
          style = MaterialTheme.typography.titleLarge,
          color = MaterialTheme.colorScheme.onPrimaryContainer,
        )
      }
    }
    Column(Modifier.weight(1f)) {
      Text(name ?: "iCloud", style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
      detail?.let {
        Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 1, overflow = TextOverflow.Ellipsis)
      }
    }
    trailing()
  }
}

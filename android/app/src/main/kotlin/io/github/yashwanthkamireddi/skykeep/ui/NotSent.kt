package io.github.yashwanthkamireddi.skykeep.ui

import android.accounts.Account
import android.provider.CalendarContract
import android.provider.ContactsContract
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.yashwanthkamireddi.skykeep.sync.NOTES_AUTHORITY
import io.github.yashwanthkamireddi.skykeep.sync.REMINDERS_AUTHORITY
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.AccountInterface
import uniffi.quince_mobile.NotSent

/** A service as the person knows it, from the engine's name for its queue. */
private fun serviceName(service: String) = when (service) {
  "contacts" -> "Contacts"
  "calendar" -> "Calendar"
  "reminders" -> "Reminders"
  "notes" -> "Notes"
  "photos" -> "Photos"
  else -> service.replaceFirstChar { it.uppercase() }
}

/** The sync that sends (and reads) a service's changes. */
private fun authorityOf(service: String) = when (service) {
  "contacts" -> ContactsContract.AUTHORITY
  "calendar" -> CalendarContract.AUTHORITY
  "reminders" -> REMINDERS_AUTHORITY
  "notes" -> NOTES_AUTHORITY
  "photos" -> io.github.yashwanthkamireddi.skykeep.sync.PHOTOS_AUTHORITY
  else -> null
}

/** Changes iCloud didn't accept, as the engine keeps them; read again whenever `key` changes. */
@Composable
fun rememberNotSent(engine: AccountInterface, key: Any?): List<NotSent> {
  var items by remember { mutableStateOf<List<NotSent>>(emptyList()) }
  LaunchedEffect(key) {
    items = withContext(Dispatchers.IO) { runCatching { engine.outboxRefused() }.getOrDefault(emptyList()) }
  }
  return items
}

/**
 * Settings' Not Sent: each change iCloud didn't accept, with what was typed and iCloud's reason,
 * kept until the person decides, as the desktop keeps them. Try Again sends it as it was made, on
 * its service's sync now; Discard lets it go, and the next sync shows iCloud's version. Nothing
 * shows when nothing waits.
 */
@Composable
fun NotSentSection(engine: AccountInterface, account: Account?) {
  val scope = rememberCoroutineScope()
  var asked by remember { mutableIntStateOf(0) }
  val items = rememberNotSent(engine, asked)
  if (items.isEmpty()) return
  fun act(n: NotSent, again: Boolean) {
    scope.launch {
      withContext(Dispatchers.IO) {
        runCatching { if (again) engine.outboxResend(n.id) else engine.outboxDiscard(n.id) }
      }
      authorityOf(n.service)?.let { authority -> account?.let { syncOne(it, authority) } }
      asked++
    }
  }
  SectionLabel("Not Sent")
  GroupCard(items, inset = 16) { n ->
    Column(Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 12.dp, bottom = 4.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
      Text(
        n.what.ifBlank { "A change in ${serviceName(n.service)}" }, style = MaterialTheme.typography.bodyLarge,
        maxLines = 2, overflow = TextOverflow.Ellipsis,
      )
      Text(
        "${serviceName(n.service)} · ${n.reason}", style = MaterialTheme.typography.bodyMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 3, overflow = TextOverflow.Ellipsis,
      )
      Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
        TextButton(onClick = { act(n, again = true) }) { Text("Try Again") }
        TextButton(onClick = { act(n, again = false) }) { Text("Discard") }
      }
    }
  }
  Text(
    "iCloud didn't accept these. Try Again sends one as you made it; Discard keeps what's in iCloud.",
    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
    modifier = Modifier.padding(horizontal = 4.dp),
  )
}

package io.github.yashwanthkamireddi.skykeep.ui

import android.Manifest
import io.github.yashwanthkamireddi.skykeep.ui.theme.LocalAccent
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.Alignment
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.Row
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.CalendarContract
import android.provider.ContactsContract
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.res.painterResource
import io.github.yashwanthkamireddi.skykeep.R
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import io.github.yashwanthkamireddi.skykeep.sync.ensureAccount
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The access the sync needs: the phone's Contacts and Calendar. */
val NEEDED = arrayOf(
  Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS,
  Manifest.permission.READ_CALENDAR, Manifest.permission.WRITE_CALENDAR,
)

fun allowed(context: Context) = NEEDED.all { ContextCompat.checkSelfPermission(context, it) == PackageManager.PERMISSION_GRANTED }

/** One fact on the first screens: a mark and a line, read as one by TalkBack. */
@Composable
private fun Fact(icon: Int, text: String) {
  Row(Modifier.fillMaxWidth().semantics(mergeDescendants = true) {}, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
    Icon(painterResource(icon), contentDescription = null, tint = LocalAccent.current, modifier = Modifier.padding(top = 2.dp).size(22.dp))
    Text(text, style = MaterialTheme.typography.bodyLarge)
  }
}

/**
 * First run, before signing in: what Skykeep does on this phone, and why it can be trusted with
 * an Apple Account, each line literally true. One action, within the thumb's reach.
 */
@Composable
fun Welcome(onContinue: () -> Unit) {
  Column(Modifier.fillMaxSize().systemBarsPadding().padding(horizontal = 28.dp).padding(top = 40.dp, bottom = 20.dp)) {
    Image(painterResource(R.mipmap.ic_launcher_foreground), contentDescription = "Skykeep", modifier = Modifier.size(112.dp).offset(x = (-20).dp))
    Spacer(Modifier.height(12.dp))
    Text("Your iCloud, on Android", style = MaterialTheme.typography.headlineLarge)
    Spacer(Modifier.height(12.dp))
    Text(
      "Reminders, Notes, Photos and iCloud Drive in Skykeep; your contacts and calendars in this phone's own apps. All kept in sync with your iPhone, iPad and Mac, both ways.",
      style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
    Spacer(Modifier.height(28.dp))
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
      Fact(R.drawable.ic_cloud_off, "Works offline. Everything opens from this phone, and changes send when you're back online.")
      Fact(R.drawable.ic_lock, "Talks only to Apple. No Skykeep account, no ads, no tracking.")
      Fact(R.drawable.ic_check_circle, "Free and open source, made by an independent developer. Not made by or affiliated with Apple.")
    }
    Spacer(Modifier.weight(1f))
    Button(onClick = onContinue, modifier = Modifier.fillMaxWidth().height(52.dp)) { Text("Get Started") }
  }
}

/**
 * After signing in: access to the phone's Contacts and Calendar, asked with its reason and a
 * Not Now that says what it means; then the first sync, each service as it arrives with what it
 * brought (the adapters' real counts, never a made-up percentage); then what happens next.
 */
@Composable
fun Setup(onDone: () -> Unit) {
  val context = LocalContext.current
  var step by remember { mutableStateOf(if (allowed(context)) 1 else 0) }
  var asked by remember { mutableLongStateOf(0L) }
  val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
    if (granted.values.all { it }) step = 1 else onDone()
  }
  Column(Modifier.fillMaxSize().systemBarsPadding().padding(horizontal = 28.dp).padding(top = 48.dp, bottom = 20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
    when (step) {
      0 -> {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
          Image(painterResource(R.drawable.app_contacts), contentDescription = null, modifier = Modifier.size(56.dp))
          Image(painterResource(R.drawable.app_calendar), contentDescription = null, modifier = Modifier.size(56.dp))
        }
        Text("Contacts and Calendar", style = MaterialTheme.typography.headlineMedium)
        Text(
          "Skykeep puts your iCloud contacts and calendars into this phone's own Contacts and Calendar apps, and sends back what you change there. Android asks you to allow that.",
          style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.weight(1f))
        Text(
          "Not now: Reminders, Notes, Photos and Drive still work, and you can allow it later from Home.",
          style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button(onClick = { ask.launch(NEEDED) }, modifier = Modifier.fillMaxWidth().height(52.dp)) { Text("Allow") }
        TextButton(onClick = onDone, modifier = Modifier.fillMaxWidth()) { Text("Not Now") }
      }
      else -> {
        var account by remember { mutableStateOf<android.accounts.Account?>(null) }
        LaunchedEffect(Unit) {
          val made = withContext(Dispatchers.IO) {
            val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
            // Named for the Apple ID, owned by the account's tag: a second Apple Account signed
            // in later gets its own phone account, and this one stops syncing (never mixed).
            val tag = engine.accountTag() ?: return@withContext null
            ensureAccount(context, engine.appleId() ?: "iCloud", tag)
          } ?: return@LaunchedEffect
          account = made
          asked = System.currentTimeMillis()
          syncNow(made)
        }
        val sync = rememberSyncView(account, asked)
        val done = AUTHORITIES.count { (sync.last[it]?.at ?: 0) >= asked && asked > 0 }
        val arrived = done == AUTHORITIES.size
        Text(if (arrived) "All set" else "Bringing in your iCloud", style = MaterialTheme.typography.headlineMedium)
        if (!arrived) {
          LinearProgressIndicator(progress = { done / AUTHORITIES.size.toFloat() }, modifier = Modifier.fillMaxWidth(), drawStopIndicator = {})
        }
        // Each service as it lands, with what it brought.
        val names = mapOf(
          ContactsContract.AUTHORITY to ("Contacts" to R.drawable.app_contacts),
          CalendarContract.AUTHORITY to ("Calendar" to R.drawable.app_calendar),
          io.github.yashwanthkamireddi.skykeep.sync.REMINDERS_AUTHORITY to ("Reminders" to R.drawable.app_reminders),
          io.github.yashwanthkamireddi.skykeep.sync.NOTES_AUTHORITY to ("Notes" to R.drawable.app_notes),
          io.github.yashwanthkamireddi.skykeep.sync.PHOTOS_AUTHORITY to ("Photos" to R.drawable.app_photos),
        )
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
          for (authority in AUTHORITIES) {
            val (name, icon) = names[authority] ?: continue
            val last = sync.last[authority]?.takeIf { it.at >= asked && asked > 0 }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp), modifier = Modifier.semantics(mergeDescendants = true) {}) {
              Image(painterResource(icon), contentDescription = null, modifier = Modifier.size(36.dp))
              Column(Modifier.weight(1f)) {
                Text(name, style = MaterialTheme.typography.titleMedium)
                Text(
                  when {
                    last == null -> "Waiting"
                    last.ok -> last.said.substringBefore(" · ")
                    else -> "Will try again"
                  },
                  style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
              }
              if (last == null) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
              else Icon(painterResource(if (last.ok) R.drawable.ic_check else R.drawable.ic_warning), contentDescription = null, tint = if (last.ok) LocalAccent.current else MaterialTheme.colorScheme.error, modifier = Modifier.size(20.dp))
            }
          }
        }
        Spacer(Modifier.weight(1f))
        Text(
          if (arrived) "From now on Skykeep syncs about every hour, and right after any change you make. Reminders due soon alert you here."
          else "You can use your phone while this finishes.",
          style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button(onClick = onDone, modifier = Modifier.fillMaxWidth().height(52.dp)) { Text(if (arrived) "Open Skykeep" else "Continue in the Background") }
      }
    }
  }
}

/** The phone's own Contacts or Calendar app. */
fun openApp(context: Context, contacts: Boolean) {
  val intent = if (contacts) {
    Intent(Intent.ACTION_VIEW, ContactsContract.Contacts.CONTENT_URI)
  } else {
    Intent(Intent.ACTION_VIEW, CalendarContract.CONTENT_URI.buildUpon().appendPath("time").build())
  }
  runCatching { context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
}

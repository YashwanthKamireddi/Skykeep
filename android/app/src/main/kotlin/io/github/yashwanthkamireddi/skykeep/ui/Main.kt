package io.github.yashwanthkamireddi.skykeep.ui

import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.semantics.semantics
import androidx.compose.material3.Surface
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.selection.toggleable
import io.github.yashwanthkamireddi.skykeep.ui.theme.mark
import io.github.yashwanthkamireddi.skykeep.ui.theme.AppTheme
import androidx.compose.foundation.Image
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.runtime.produceState
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.IconButton
import androidx.compose.ui.text.style.TextOverflow
import io.github.yashwanthkamireddi.skykeep.ui.theme.Palette
import android.accounts.Account
import android.content.ContentResolver
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.CalendarContract
import android.provider.CalendarContract.Calendars
import android.provider.ContactsContract
import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import io.github.yashwanthkamireddi.skykeep.R
import io.github.yashwanthkamireddi.skykeep.sync.Outcome
import io.github.yashwanthkamireddi.skykeep.sync.SignOut
import io.github.yashwanthkamireddi.skykeep.sync.setSyncEvery
import io.github.yashwanthkamireddi.skykeep.sync.syncEvery
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The bottom bar: five places, as Material's navigation bar holds (three to five). Contacts and
 * Calendar live in the phone's own apps; their settings open from their cards on Home.
 */
private enum class Tab(val label: String, val icon: Int, val app: Palette.App?) {
  Home("Home", R.drawable.ic_home, null),
  Reminders("Reminders", R.drawable.ic_reminders, Palette.App.Reminders),
  Notes("Notes", R.drawable.ic_notes, Palette.App.Notes),
  Photos("Photos", R.drawable.ic_photos, Palette.App.Photos),
  Drive("Drive", R.drawable.ic_drive, Palette.App.Drive),
}

/** A page opened from Home rather than the bar: Contacts' or Calendar's settings. */
private enum class Page { Contacts, Calendar, Settings, Licences }

/**
 * Skykeep once set up. The using happens in the phone's own Contacts and Calendar apps; here are
 * the account, how syncing is going, and what syncs — management screens, as the design has it,
 * and saying so plainly.
 */
@OptIn(androidx.compose.material3.ExperimentalMaterial3Api::class)
@Composable
fun MainScreens(engine: uniffi.quince_mobile.AccountInterface, onSignedOut: () -> Unit) {
  val context = LocalContext.current
  // Kept through a rotation or a light/dark switch (Android rebuilds the screen for both).
  var tab by rememberSaveable { mutableStateOf(Tab.Home) }
  var page by rememberSaveable { mutableStateOf<Page?>(null) }
  // Back goes up one level: a page to Home, another tab to Home, then out of the app.
  androidx.activity.compose.BackHandler(enabled = page != null || tab != Tab.Home) { if (page != null) page = null else tab = Tab.Home }
  var asked by remember { mutableLongStateOf(0L) }
  val account = remember { phoneAccount(context) }
  val sync = rememberSyncView(account, asked)
  fun now() { account?.let { asked = System.currentTimeMillis(); syncNow(it) } }
  val full = remember { mutableStateOf(false) }
  androidx.compose.runtime.CompositionLocalProvider(LocalFullScreen provides full) {
  Scaffold(
    bottomBar = {
      if (!full.value) NavigationBar {
        for (t in Tab.entries) {
          // Each app's mark in its own colour, as the desktop's sidebar shows them.
          val tint = t.app?.let { mark(it) } ?: MaterialTheme.colorScheme.onSurfaceVariant
          NavigationBarItem(
            selected = tab == t,
            onClick = { tab = t; page = null },
            icon = { Icon(painterResource(t.icon), contentDescription = null) },
            label = { Text(t.label) },
            colors = NavigationBarItemDefaults.colors(
              selectedIconColor = t.app?.let { mark(it) } ?: MaterialTheme.colorScheme.onSurface,
              unselectedIconColor = tint,
              selectedTextColor = MaterialTheme.colorScheme.onSurface,
              indicatorColor = (t.app?.let { mark(it) } ?: MaterialTheme.colorScheme.onSurface).copy(alpha = 0.14f),
            ),
          )
        }
      }
    },
  ) { inner ->
    // Reminders, Notes and Photos scroll their own lists (they can be long); the rest are one column.
    if (page == Page.Licences) {
      Box(Modifier.padding(inner).fillMaxSize()) { LicencesPage { page = Page.Settings } }
    } else if (page == null && (tab == Tab.Reminders || tab == Tab.Notes || tab == Tab.Photos || tab == Tab.Drive)) {
      Box(Modifier.padding(inner).fillMaxSize()) {
        when (tab) {
          Tab.Notes -> AppTheme(Palette.App.Notes) { NotesPage(engine) }
          Tab.Photos -> PhotosPage(engine)
          Tab.Drive -> DrivePage(engine)
          else -> RemindersPage(engine)
        }
      }
    } else {
      // Home pulls down to sync (the same as its Sync button); the indicator shows only for a
      // sync asked for here, until each service has answered.
      PullToRefreshBox(
        isRefreshing = asked > 0 && sync.busy,
        onRefresh = ::now,
        modifier = Modifier.padding(inner).fillMaxSize(),
      ) {
        Box(Modifier.fillMaxSize().verticalScroll(rememberScrollState())) {
          Column(Modifier.padding(horizontal = 20.dp, vertical = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            if (page != null) IconButton(onClick = { page = null }, modifier = Modifier.padding(start = 0.dp)) {
              Icon(painterResource(R.drawable.ic_back), contentDescription = "Home")
            }
            when {
              page == Page.Contacts -> ContactsPage(account)
              page == Page.Calendar -> AppTheme(Palette.App.Calendar) { CalendarPage(account) }
              page == Page.Settings -> SettingsPage(engine, account, onSignedOut) { page = Page.Licences }
              tab == Tab.Home -> Home(engine, account, sync, onSync = ::now, onOpen = { page = it }, onTab = { tab = it; page = null })
            }
          }
        }
      }
    }
  }
  }
}

/** One service as Home lists it: its sync authority, name, mark and colour. */
private data class Service(val authority: String, val name: String, val icon: Int)

private val SERVICES = listOf(
  Service(io.github.yashwanthkamireddi.skykeep.sync.REMINDERS_AUTHORITY, "Reminders", R.drawable.app_reminders),
  Service(io.github.yashwanthkamireddi.skykeep.sync.NOTES_AUTHORITY, "Notes", R.drawable.app_notes),
  Service(io.github.yashwanthkamireddi.skykeep.sync.PHOTOS_AUTHORITY, "Photos", R.drawable.app_photos),
  Service(ContactsContract.AUTHORITY, "Contacts", R.drawable.app_contacts),
  Service(CalendarContract.AUTHORITY, "Calendar", R.drawable.app_calendar),
)

/** Something on a card the person should act on or know: a failed sync, or items only here. */
private fun Outcome.Last?.needsYou() = this != null && (!ok || paused || onlyHere > 0)

/**
 * Home: the account, then one card per service, the ones that need the person first. Each card
 * says its own state in the same few calm words (Up to date, Syncing, Will try again, Sign in
 * again); nothing spins for the whole screen. Sync is in the top bar and on a pull.
 */
@Composable
private fun Home(
  engine: uniffi.quince_mobile.AccountInterface, account: Account?, sync: SyncView,
  onSync: () -> Unit, onOpen: (Page) -> Unit, onTab: (Tab) -> Unit,
) {
  val context = LocalContext.current
  val online = rememberOnline()
  // "2 minutes ago" stays true while Home is open: the time moves on each minute.
  val now by produceState(System.currentTimeMillis()) {
    while (true) { kotlinx.coroutines.delay(60_000); value = System.currentTimeMillis() }
  }
  // Read again after each sync: the session (name, address) is open by then, and Contacts
  // brings the person's own card and its picture.
  AccountRow(engine, account, key = sync.last.values.maxOfOrNull { it?.at ?: 0L }) {
    IconButton(onClick = onSync, enabled = account != null && !sync.busy && online) {
      Icon(painterResource(R.drawable.ic_sync), contentDescription = "Sync now")
    }
    IconButton(onClick = { onOpen(Page.Settings) }) {
      Icon(painterResource(R.drawable.ic_settings), contentDescription = "Settings")
    }
  }
  if (!online) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
      Icon(painterResource(R.drawable.ic_cloud_off), contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(18.dp))
      Text(
        "Offline. Everything here is on this phone, and changes you make will send when you're back online.",
        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
      )
    }
  }
  if (!allowed(context)) {
    val ask = androidx.activity.compose.rememberLauncherForActivityResult(
      androidx.activity.result.contract.ActivityResultContracts.RequestMultiplePermissions(),
    ) { granted -> if (granted.values.all { it }) onSync() }
    Notice("Allow Contacts and Calendar so Skykeep can add your iCloud contacts and calendars to this phone.") {
      Button(onClick = { ask.launch(NEEDED) }) { Text("Allow") }
    }
  }
  if (sync.held > 0 && !sync.busy) {
    Notice("${sync.held} changes are waiting for you. More than usual changed at once, so Skykeep held them to be safe.") {
      Button(onClick = {
        AUTHORITIES.forEach { Outcome.confirmMassRemoval(context, it) }
        onSync()
      }) { Text("Apply Them") }
    }
  }
  DataProtection(engine, sync.last, onApproved = onSync)
  // Changes iCloud didn't accept wait in Settings for the person to decide; said here once, calmly,
  // and read again after each sync.
  val notSent = rememberNotSent(engine, sync.last.values.maxOfOrNull { it?.at ?: 0L })
  if (notSent.isNotEmpty()) {
    Notice(
      if (notSent.size == 1) "1 change wasn't accepted by iCloud. It's kept for you in Settings."
      else "${notSent.size} changes weren't accepted by iCloud. They're kept for you in Settings.",
    ) {
      TextButton(onClick = { onOpen(Page.Settings) }) { Text("Review") }
    }
  }
  // Phone makers' battery savers pause apps that aren't opened often, and a sync with them. Said
  // only when it has happened: nothing synced cleanly for two days, though Skykeep has synced
  // before.
  val lastGood = sync.last.values.mapNotNull { it?.okAt?.takeIf { t -> t > 0 } }.maxOrNull()
  if (lastGood != null && System.currentTimeMillis() - lastGood > 48 * 3_600_000L) {
    val brand = android.os.Build.MANUFACTURER.replaceFirstChar { it.uppercase() }
    Notice(
      "Skykeep hasn't synced for two days. $brand phones can pause apps that aren't opened often. Allow Skykeep to run in the background so everything stays up to date.",
      warning = true,
    ) {
      Button(onClick = {
        runCatching {
          context.startActivity(Intent(android.provider.Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
      }) { Text("Battery Settings") }
      TextButton(onClick = {
        context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://dontkillmyapp.com/${android.os.Build.MANUFACTURER.lowercase()}")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
      }) { Text("How on $brand") }
    }
  }
  // Problems float up; otherwise the order people use them in.
  for (s in SERVICES.sortedBy { if (sync.last[it.authority].needsYou()) 0 else 1 }) {
    val open: () -> Unit = when (s.name) {
      "Contacts" -> { { onOpen(Page.Contacts) } }
      "Calendar" -> { { onOpen(Page.Calendar) } }
      "Reminders" -> { { onTab(Tab.Reminders) } }
      "Notes" -> { { onTab(Tab.Notes) } }
      else -> { { onTab(Tab.Photos) } }
    }
    Health(s, sync.last[s.authority], syncing = s.authority in sync.active, firstRun = sync.busy, now = now, onClick = open)
  }
  DriveCard(engine, online) { onTab(Tab.Drive) }
}

/**
 * Drive on Home: it has no sync of its own (folders are read as they're opened, files fetched as
 * they're used), so its card says how much of iCloud is used, the one number that matters here.
 */
@Composable
private fun DriveCard(engine: uniffi.quince_mobile.AccountInterface, online: Boolean, onClick: () -> Unit) {
  val context = LocalContext.current
  // Asked again when the phone comes back online: offline, Apple can't say.
  val space by produceState<List<ULong>?>(null, online) {
    if (online) value = runCatching { kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { engine.driveSpace() } }.getOrNull()
  }
  Surface(
    onClick = onClick, color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large,
    modifier = Modifier.fillMaxWidth().semantics(mergeDescendants = true) {},
  ) {
    Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
      Image(painterResource(R.drawable.app_drive), contentDescription = null, modifier = Modifier.size(48.dp))
      Column(Modifier.weight(1f)) {
        Text("Drive", style = MaterialTheme.typography.titleMedium)
        Text(
          space?.let { (used, total) ->
            "${android.text.format.Formatter.formatShortFileSize(context, used.toLong())} of ${android.text.format.Formatter.formatShortFileSize(context, total.toLong())} used"
          } ?: "Files open as you need them",
          style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text("Also in the Files app", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
      }
      Icon(painterResource(R.drawable.ic_chevron_right), contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
  }
}

/** The app's names for the services Advanced Data Protection holds, by the engine's. */
private val PROTECTED = mapOf("notes" to "Notes", "reminders" to "Reminders", "photos" to "Photos", "drive" to "Drive")

/** "Notes", "Notes and Photos", "Notes, Reminders and Photos". */
private fun spoken(names: List<String>): String =
  if (names.size <= 1) names.firstOrNull() ?: "Your data" else names.dropLast(1).joinToString(", ") + " and " + names.last()

/**
 * Advanced Data Protection, as the desktop asks for it (data_access.rs): with it on, Apple lets
 * this phone read Notes, Reminders, Photos and Drive only once the person approves on one of
 * their own devices, for about an hour at a time. Background syncs never ask (each request
 * shows a notice on their devices), and while it waits those services aren't asked of Apple at
 * all; this is the one place it is asked for, when the person presses the button. One approval
 * covers every service; once it lands, they sync.
 */
@Composable
private fun DataProtection(engine: uniffi.quince_mobile.AccountInterface, last: Map<String, Outcome.Last?>, onApproved: () -> Unit) {
  val scope = rememberCoroutineScope()
  var now by remember { mutableStateOf<uniffi.quince_mobile.DataAccessNow?>(null) }
  var asking by remember { mutableStateOf(false) }
  // From what the phone holds alone, read again whenever a sync says something new.
  LaunchedEffect(last) { now = withContext(Dispatchers.IO) { runCatching { engine.dataAccessStanding() }.getOrNull() } }
  val n = now ?: return
  if (n.state == uniffi.quince_mobile.DataState.OPEN && !asking) return
  val what = spoken(n.services.mapNotNull { PROTECTED[it] })
  val text = when {
    asking || n.state == uniffi.quince_mobile.DataState.WAITING ->
      "Check your iPhone, iPad or Mac and allow Skykeep. $what will sync as soon as you do."
    n.state == uniffi.quince_mobile.DataState.WEB_ACCESS_OFF ->
      "Advanced Data Protection is on, and Access iCloud Data on the Web is off. Turn it on in iCloud settings on your iPhone to sync here."
    n.state == uniffi.quince_mobile.DataState.NOT_SENT ->
      "Apple couldn't show the request on your devices. Make sure your iPhone is signed in to iCloud, then try again."
    else -> "Advanced Data Protection keeps $what until you approve Skykeep on one of your devices. Everything on this phone stays."
  } + (n.problem?.let { " $it" } ?: "")
  Notice(text) {
    if (n.state != uniffi.quince_mobile.DataState.WEB_ACCESS_OFF) {
      Button(enabled = !asking, onClick = {
        scope.launch {
          asking = true
          var step = withContext(Dispatchers.IO) { engine.dataAccessAsk() }
          now = step
          // Approved on a device: followed until the keys are here (five minutes at most).
          if (step.state == uniffi.quince_mobile.DataState.WAITING) {
            step = withContext(Dispatchers.IO) { engine.dataAccessFollow() }
            now = step
          }
          asking = false
          if (step.state == uniffi.quince_mobile.DataState.OPEN) onApproved()
        }
      }) { Text(if (asking) "Waiting for Approval" else "Approve on your iPhone") }
    }
  }
}

/** One service's sync health, in the design's words, as one stop for TalkBack. */
@Composable
private fun Health(s: Service, last: Outcome.Last?, syncing: Boolean, firstRun: Boolean, now: Long, onClick: () -> Unit) {
  val colours = MaterialTheme.colorScheme
  val signIn = last != null && !last.ok &&
    (last.said.contains("session", ignoreCase = true) || last.said.contains("sign in", ignoreCase = true))
  val (status, trouble) = when {
    syncing -> "Syncing" to false
    last == null && firstRun -> "Setting up" to false
    last == null -> "Not synced yet" to false
    // Advanced Data Protection: waiting on the person, not failing (the approval is above).
    last.paused -> "Waiting for your approval" to false
    last.ok -> "Up to date · ${ago(last.at, now)}" to false
    signIn -> "Sign in again" to true
    else -> "Will try again" to true
  }
  val detail = when {
    last == null && firstRun -> "The first sync can take a little while. You can keep using your phone."
    last == null -> null
    last.paused -> "What's on this phone stays, and changes you make here wait to be sent." +
      (if (last.okAt > 0) " Last synced ${ago(last.okAt, now)}." else "")
    last.ok -> null
    signIn -> "Your Apple sign-in has expired. Nothing is lost, and changes waiting to send are kept."
    else -> "Apple didn't answer. Skykeep will try again on its own." + (if (last.okAt > 0) " Last synced ${ago(last.okAt, now)}." else "")
  }
  // What the adapter counted ("124 contacts"), without its per-sync change tally.
  val counts = last?.takeIf { it.ok && !it.paused }?.said?.substringBefore(" · ")
  Surface(
    onClick = onClick,
    color = colours.surfaceContainer,
    shape = MaterialTheme.shapes.large,
    modifier = Modifier.fillMaxWidth().semantics(mergeDescendants = true) {},
  ) {
    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
      Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
        Image(painterResource(s.icon), contentDescription = null, modifier = Modifier.size(48.dp))
        Column(Modifier.weight(1f)) {
          Text(s.name, style = MaterialTheme.typography.titleMedium)
          Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            if (trouble) Icon(painterResource(R.drawable.ic_warning), contentDescription = null, tint = colours.error, modifier = Modifier.size(16.dp))
            Text(
              status, style = MaterialTheme.typography.bodyMedium,
              color = if (trouble) colours.error else colours.onSurfaceVariant,
            )
          }
          counts?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = colours.onSurfaceVariant) }
        }
        Icon(painterResource(R.drawable.ic_chevron_right), contentDescription = null, tint = colours.onSurfaceVariant)
      }
      detail?.let { Text(it, style = MaterialTheme.typography.bodyMedium, color = colours.onSurfaceVariant) }
      if (last != null && last.onlyHere > 0) {
        val what = if (s.name == "Contacts") "contact" else "event"
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
          Icon(painterResource(R.drawable.ic_warning), contentDescription = null, tint = colours.error, modifier = Modifier.size(18.dp))
          Text(
            (if (last.onlyHere == 1) "1 $what is only on this phone." else "${last.onlyHere} ${what}s are only on this phone.") +
              (last.why?.let { " $it" } ?: ""),
            style = MaterialTheme.typography.bodyMedium, color = colours.error,
          )
        }
      }
    }
  }
}

@Composable
private fun ContactsPage(account: Account?) {
  val context = LocalContext.current
  Text("Contacts", style = MaterialTheme.typography.headlineSmall)
  Text(
    "Your iCloud contacts are in this phone's Contacts app, under Skykeep. Change them there, and the change reaches iCloud and your other devices.",
    style = MaterialTheme.typography.bodyMedium,
  )
  val count = remember {
    account?.let {
      context.contentResolver.query(
        ContactsContract.RawContacts.CONTENT_URI, arrayOf(ContactsContract.RawContacts._ID),
        "${ContactsContract.RawContacts.ACCOUNT_TYPE}=? AND ${ContactsContract.RawContacts.ACCOUNT_NAME}=? AND ${ContactsContract.RawContacts.DELETED}=0",
        arrayOf(it.type, it.name), null,
      )?.use { c -> c.count }
    } ?: 0
  }
  SyncSwitch(account, ContactsContract.AUTHORITY, "Sync contacts", "$count contacts on this phone")
  Card(colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceVariant)) {
    Text(
      "New contacts you create may go to a different account. In your contacts app, choose Skykeep as where new contacts are saved, so they reach iCloud.",
      style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(16.dp),
    )
  }
  FilledTonalButton(onClick = { openApp(context, contacts = true) }) { Text("Open Contacts") }
}

private data class PhoneCalendar(val id: Long, val name: String, val colour: Int, val visible: Boolean, val readOnly: Boolean)

@Composable
private fun CalendarPage(account: Account?) {
  val context = LocalContext.current
  Text("Calendar", style = MaterialTheme.typography.headlineSmall)
  Text(
    "Your iCloud calendars are in this phone's Calendar app. Events you add or change there reach iCloud and your other devices.",
    style = MaterialTheme.typography.bodyMedium,
  )
  SyncSwitch(account, CalendarContract.AUTHORITY, "Sync calendars", "A month back and a year ahead")
  val calendars = remember { mutableStateListOf<PhoneCalendar>() }
  LaunchedEffect(account) {
    calendars.clear()
    calendars += withContext(Dispatchers.IO) { account?.let { phoneCalendars(context, it) } ?: emptyList() }
  }
  if (calendars.isNotEmpty()) {
    Text("Shown on this phone", style = MaterialTheme.typography.titleSmall)
    for ((i, k) in calendars.withIndex()) {
      ListItem(
        headlineContent = { Text(k.name) },
        supportingContent = if (k.readOnly) ({ Text("Read only") }) else null,
        leadingContent = { Box(Modifier.size(14.dp).background(Color(k.colour), CircleShape)) },
        trailingContent = {
          Switch(checked = k.visible, onCheckedChange = { on ->
            calendars[i] = k.copy(visible = on)
            showCalendar(context, account!!, k.id, on)
          })
        },
      )
    }
    Text(
      "Which calendars show is set here for this phone. Colours come from iCloud.",
      style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant,
    )
  }
  FilledTonalButton(onClick = { openApp(context, contacts = false) }) { Text("Open Calendar") }
}

/** Whether Android syncs this authority on its own (and on a change made on the phone). */
@Composable
private fun SyncSwitch(account: Account?, authority: String, title: String, detail: String) {
  var on by remember { mutableStateOf(account?.let { ContentResolver.getSyncAutomatically(it, authority) } ?: false) }
  ListItem(
    headlineContent = { Text(title) },
    supportingContent = { Text(detail) },
    trailingContent = {
      Switch(checked = on, enabled = account != null, onCheckedChange = { v ->
        on = v
        account?.let { ContentResolver.setSyncAutomatically(it, authority, v) }
      })
    },
  )
}

@Composable
private fun SettingsPage(engine: uniffi.quince_mobile.AccountInterface, account: Account?, onSignedOut: () -> Unit, onLicences: () -> Unit) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  Text("Settings", style = MaterialTheme.typography.headlineLarge)
  Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large) {
    Box(Modifier.fillMaxWidth().padding(16.dp)) { AccountRow(engine, account) }
  }
  // Off unless turned on; from that moment on, never the whole camera roll at once.
  var backup by remember { mutableStateOf(io.github.yashwanthkamireddi.skykeep.sync.PhotoBackup.on(context)) }
  // The photos to read, and where each was taken: without ACCESS_MEDIA_LOCATION Android hands
  // over photos with their location removed, and the backup in iCloud would be missing it.
  val media = (if (android.os.Build.VERSION.SDK_INT >= 33) arrayOf(android.Manifest.permission.READ_MEDIA_IMAGES, android.Manifest.permission.READ_MEDIA_VIDEO)
    else arrayOf(android.Manifest.permission.READ_EXTERNAL_STORAGE)) + android.Manifest.permission.ACCESS_MEDIA_LOCATION
  val askMedia = androidx.activity.compose.rememberLauncherForActivityResult(androidx.activity.result.contract.ActivityResultContracts.RequestMultiplePermissions()) { granted ->
    // Location is the person's choice: backup runs without it (photos then arrive without where).
    if (granted.filterKeys { it != android.Manifest.permission.ACCESS_MEDIA_LOCATION }.values.all { it }) {
      backup = true; io.github.yashwanthkamireddi.skykeep.sync.PhotoBackup.set(context, true)
    }
  }
  var cellular by remember { mutableStateOf(io.github.yashwanthkamireddi.skykeep.sync.PhotoBackup.mobileData(context)) }
  var charging by remember { mutableStateOf(io.github.yashwanthkamireddi.skykeep.sync.PhotoBackup.whileCharging(context)) }
  SectionLabel("Photos")
  Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large) {
    Column {
      SwitchRow("Back up new photos", "Photos and videos you take on this phone from now on go to iCloud Photos.", backup) { on ->
        if (on) askMedia.launch(media) else { backup = false; io.github.yashwanthkamireddi.skykeep.sync.PhotoBackup.set(context, false) }
      }
      if (backup) {
        HorizontalDivider(Modifier.padding(start = 16.dp), color = MaterialTheme.colorScheme.outlineVariant)
        SwitchRow("Use mobile data", "Off: photos wait for Wi-Fi.", cellular) { cellular = it; io.github.yashwanthkamireddi.skykeep.sync.PhotoBackup.setMobileData(context, it) }
        HorizontalDivider(Modifier.padding(start = 16.dp), color = MaterialTheme.colorScheme.outlineVariant)
        SwitchRow("Only while charging", "Photos wait until this phone is charging.", charging) { charging = it; io.github.yashwanthkamireddi.skykeep.sync.PhotoBackup.setWhileCharging(context, it) }
      }
    }
  }
  SectionLabel("Sync")
  var every by remember { mutableLongStateOf(syncEvery(context)) }
  val choices = listOf(3_600L to "About every hour", 3 * 3_600L to "About every 3 hours", 6 * 3_600L to "About every 6 hours", 12 * 3_600L to "About every 12 hours", 86_400L to "About once a day")
  GroupCard(choices, inset = 16) { (seconds, label) ->
    Row(
      Modifier.fillMaxWidth().selectable(selected = every == seconds, role = Role.RadioButton, onClick = {
        every = seconds
        setSyncEvery(context, account, seconds)
      }).padding(horizontal = 16.dp, vertical = 12.dp),
      verticalAlignment = Alignment.CenterVertically,
    ) {
      Text(label, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
      RadioButton(selected = every == seconds, onClick = null)
    }
  }
  Text(
    "Android picks the exact moment, to save battery. A change made on this phone is sent at once, whatever this is set to.",
    style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(horizontal = 4.dp),
  )
  NotSentSection(engine, account)
  SectionLabel("Privacy")
  Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large) {
    Text(
      "Skykeep talks only to Apple: no Skykeep account, no ads, no analytics, no crash reports. It signs in the way Apple's own sign-in does (SRP), proving your password to Apple without sending it. To stay signed in, it keeps your password and session sealed with a key in this phone's Android Keystore, left out of backups, and erases them when you sign out. Your library is stored in this app's own storage on this phone.",
      style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(16.dp),
    )
  }
  SectionLabel("About")
  val version = remember { runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() }
  Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large) {
    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
      Text("Skykeep${version?.let { " $it" } ?: ""}", style = MaterialTheme.typography.titleMedium)
      Text("Free and open source (GPL-3.0), made by an independent developer. Not made by or affiliated with Apple. iCloud, iPhone and Apple are trademarks of Apple Inc.", style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
      TextButton(onClick = {
        context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://github.com/YashwanthKamireddi/Skykeep")))
      }, contentPadding = PaddingValues(0.dp)) { Text("Source code on GitHub") }
      TextButton(onClick = onLicences, contentPadding = PaddingValues(0.dp)) { Text("Open-source licences") }
    }
  }
  Spacer(Modifier.height(8.dp))
  var leaving by remember { mutableStateOf(false) }
  var unsent by remember { mutableIntStateOf(0) }
  var working by remember { mutableStateOf(false) }
  var trouble by remember { mutableStateOf<String?>(null) }
  OutlinedButton(
    modifier = Modifier.fillMaxWidth(),
    colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.error),
    enabled = !working,
    onClick = {
      scope.launch {
        // Everything not yet at Apple: the phone's contacts and calendars, and the notes,
        // reminders and photo changes waiting in the engine's library.
        unsent = withContext(Dispatchers.IO) {
          (account?.let { SignOut.unsent(context.contentResolver, it) } ?: 0) +
            (runCatching { engine.unsentChanges().toInt() }.getOrDefault(0))
        }
        leaving = true
      }
    },
  ) { Text("Sign Out") }
  trouble?.let { Text(it, color = MaterialTheme.colorScheme.error) }
  if (leaving) {
    AlertDialog(
      onDismissRequest = { leaving = false },
      title = { Text("Sign out of iCloud on this phone?") },
      text = {
        Text(
          (if (unsent > 0) "$unsent changes made on this phone haven't reached iCloud yet and won't be sent.\n\n" else "") +
            "Keep a copy and your contacts and calendars stay on this phone as its own, no longer synced. iCloud itself isn't changed either way.",
        )
      },
      // Three choices, stacked as Material lays out more than two: the safe one first, the one
      // that removes in the error colour, Cancel last.
      confirmButton = {
        Column(horizontalAlignment = Alignment.End) {
          for ((label, keep) in listOf("Keep a Copy" to true, "Remove from Phone" to false)) {
            TextButton(
              onClick = {
                leaving = false
                working = true
                scope.launch {
                  val done = withContext(Dispatchers.IO) {
                    runCatching {
                      account?.let { a ->
                        if (keep) SignOut.keepCopy(context.contentResolver, a)
                        SignOut.removeAccount(context, a)
                      }
                      // Remove from Phone: the account's library and Skykeep's own copies go too,
                      // as the desktop's Remove does; nothing of it is left in the app's storage.
                      if (!keep) { engine.forgetAccount(); SignOut.forgetHere(context) }
                      engine.signOut()
                    }
                  }
                  working = false
                  done.onSuccess { onSignedOut() }.onFailure { trouble = "Couldn't sign out: ${it.message}" }
                }
              },
              colors = if (keep) ButtonDefaults.textButtonColors() else ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error),
            ) { Text(label) }
          }
          TextButton(onClick = { leaving = false }) { Text("Cancel") }
        }
      },
    )
  }
}

/** A setting that is on or off: its name, what it means, and Android's switch; the whole row toggles. */
@Composable
private fun SwitchRow(title: String, detail: String, on: Boolean, onChange: (Boolean) -> Unit) {
  Row(
    Modifier.fillMaxWidth().toggleable(on, role = Role.Switch, onValueChange = onChange).padding(horizontal = 16.dp, vertical = 12.dp),
    verticalAlignment = Alignment.CenterVertically,
  ) {
    Column(Modifier.weight(1f)) {
      Text(title, style = MaterialTheme.typography.bodyLarge)
      Text(detail, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
    Spacer(Modifier.width(12.dp))
    androidx.compose.material3.Switch(on, onCheckedChange = null)
  }
}

private fun phoneCalendars(context: Context, account: Account): List<PhoneCalendar> {
  val out = ArrayList<PhoneCalendar>()
  context.contentResolver.query(
    Calendars.CONTENT_URI,
    arrayOf(Calendars._ID, Calendars.CALENDAR_DISPLAY_NAME, Calendars.CALENDAR_COLOR, Calendars.VISIBLE, Calendars.CALENDAR_ACCESS_LEVEL),
    "${Calendars.ACCOUNT_TYPE}=? AND ${Calendars.ACCOUNT_NAME}=?", arrayOf(account.type, account.name),
    "${Calendars.CALENDAR_DISPLAY_NAME} COLLATE NOCASE",
  )?.use { c ->
    while (c.moveToNext()) {
      out += PhoneCalendar(c.getLong(0), c.getString(1) ?: "", c.getInt(2), c.getInt(3) != 0, c.getInt(4) < Calendars.CAL_ACCESS_CONTRIBUTOR)
    }
  }
  return out
}

/** A calendar shown or hidden on this phone: a setting of the phone's, never sent to iCloud. */
private fun showCalendar(context: Context, account: Account, id: Long, visible: Boolean) {
  val uri = Calendars.CONTENT_URI.buildUpon()
    .appendQueryParameter(CalendarContract.CALLER_IS_SYNCADAPTER, "true")
    .appendQueryParameter(Calendars.ACCOUNT_NAME, account.name)
    .appendQueryParameter(Calendars.ACCOUNT_TYPE, account.type)
    .build()
  context.contentResolver.update(uri, ContentValues().apply { put(Calendars.VISIBLE, if (visible) 1 else 0) }, "${Calendars._ID}=?", arrayOf(id.toString()))
}

/** "5 minutes ago", in the phone's language. */
/** "just now" under a minute (Android's own words would be "0 minutes ago"), then "5 minutes ago". */
private fun ago(ms: Long, now: Long = System.currentTimeMillis()): String =
  if (now - ms < DateUtils.MINUTE_IN_MILLIS) "just now"
  else DateUtils.getRelativeTimeSpanString(ms, now, DateUtils.MINUTE_IN_MILLIS).toString().lowercase()

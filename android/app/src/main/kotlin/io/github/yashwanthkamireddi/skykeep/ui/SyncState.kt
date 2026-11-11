package io.github.yashwanthkamireddi.skykeep.ui

import android.accounts.Account
import android.content.ContentResolver
import android.content.Context
import android.os.Bundle
import android.provider.CalendarContract
import android.provider.ContactsContract
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import io.github.yashwanthkamireddi.skykeep.sync.Outcome
import io.github.yashwanthkamireddi.skykeep.sync.SyncWork

/** Contacts and Calendar, the two things the phone's own apps show from iCloud. */
val AUTHORITIES = listOf(ContactsContract.AUTHORITY, CalendarContract.AUTHORITY, io.github.yashwanthkamireddi.skykeep.sync.REMINDERS_AUTHORITY, io.github.yashwanthkamireddi.skykeep.sync.NOTES_AUTHORITY, io.github.yashwanthkamireddi.skykeep.sync.PHOTOS_AUTHORITY)

/** The Skykeep account on this phone, once the first sync has made it. */
fun phoneAccount(context: Context): Account? = io.github.yashwanthkamireddi.skykeep.sync.currentAccount(context)

/** Android asked to sync now — the adapters are the one place a sync runs. */
fun syncNow(account: Account) {
  val now = Bundle().apply {
    putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
    putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
  }
  for (authority in AUTHORITIES) ContentResolver.requestSync(account, authority, now)
}

/** One service's sync, now: a change sent again from Settings goes at once. */
fun syncOne(account: Account, authority: String) {
  val now = Bundle().apply {
    putBoolean(ContentResolver.SYNC_EXTRAS_MANUAL, true)
    putBoolean(ContentResolver.SYNC_EXTRAS_EXPEDITED, true)
  }
  ContentResolver.requestSync(account, authority, now)
}

/** What the screens show about syncing, as it is now. */
data class SyncView(
  /** A sync is running, or one asked for here hasn't answered yet. */
  val busy: Boolean,
  /** Each authority's last outcome, if it has synced at all. */
  val last: Map<String, Outcome.Last?>,
  /** Changes held by the circuit breaker, waiting for the person to confirm. */
  val held: Int,
  /** The authorities Android is syncing at this moment. */
  val active: Set<String> = emptySet(),
)

/**
 * The sync state, kept current: told by Android when a sync starts or stops, and by the adapters
 * when one finishes. `asked` is when a sync was last asked for here: busy until each authority
 * has answered since, or two minutes pass (never stuck on an answer that doesn't come).
 */
@Composable
fun rememberSyncView(account: Account?, asked: Long): SyncView {
  val context = LocalContext.current
  // Syncing on the interval runs as WorkManager work, which Android's sync status doesn't see:
  // the services whose sync work is running, as WorkManager says.
  var working by remember { mutableStateOf(emptySet<String>()) }
  fun read(): SyncView {
    val last = AUTHORITIES.associateWith { Outcome.last(context, it) }
    val waiting = asked > 0 && System.currentTimeMillis() - asked < 120_000 &&
      last.values.any { (it?.at ?: 0) < asked }
    val active = if (account == null) emptySet() else AUTHORITIES.filter { ContentResolver.isSyncActive(account, it) || it in working }.toSet()
    return SyncView(waiting || active.isNotEmpty(), last, AUTHORITIES.sumOf { Outcome.held(context, it) }, active)
  }
  var view by remember(account, asked) { mutableStateOf(read()) }
  androidx.compose.runtime.LaunchedEffect(Unit) {
    androidx.work.WorkManager.getInstance(context).getWorkInfosByTagFlow(SyncWork.SYNCING).collect { infos ->
      working = infos.filter { it.state == androidx.work.WorkInfo.State.RUNNING }
        .flatMap { info -> AUTHORITIES.filter { SyncWork.authorityTag(it) in info.tags } }.toSet()
      view = read()
    }
  }
  DisposableEffect(account, asked) {
    val handle = ContentResolver.addStatusChangeListener(ContentResolver.SYNC_OBSERVER_TYPE_ACTIVE) {
      android.os.Handler(android.os.Looper.getMainLooper()).post { view = read() }
    }
    val unwatch = Outcome.watch(context) { view = read() }
    val tick = android.os.Handler(android.os.Looper.getMainLooper())
    // The two-minute limit on waiting passes with no event to say so.
    val check = object : Runnable {
      override fun run() { view = read(); if (asked > 0) tick.postDelayed(this, 5_000) }
    }
    tick.postDelayed(check, 5_000)
    onDispose {
      ContentResolver.removeStatusChangeListener(handle)
      unwatch()
      tick.removeCallbacks(check)
    }
  }
  return view
}

package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.AbstractAccountAuthenticator
import android.accounts.Account
import android.accounts.AccountAuthenticatorResponse
import android.app.Service
import android.content.AbstractThreadedSyncAdapter
import android.content.ContentProviderClient
import android.content.Context
import android.content.Intent
import android.content.SyncResult
import android.os.Bundle
import android.os.IBinder
import kotlin.time.Duration.Companion.minutes
import kotlinx.coroutines.runBlocking

/**
 * Skykeep's account as Android's account system sees it. Accounts are added by the app after
 * signing in, never from Settings' Add account, so every request here is declined plainly.
 */
private class Authenticator(context: Context) : AbstractAccountAuthenticator(context) {
  override fun editProperties(r: AccountAuthenticatorResponse?, t: String?) = null
  override fun addAccount(r: AccountAuthenticatorResponse?, t: String?, a: String?, f: Array<out String>?, o: Bundle?) = null
  override fun confirmCredentials(r: AccountAuthenticatorResponse?, a: Account?, o: Bundle?) = null
  override fun getAuthToken(r: AccountAuthenticatorResponse?, a: Account?, t: String?, o: Bundle?) = null
  override fun getAuthTokenLabel(t: String?) = null
  override fun updateCredentials(r: AccountAuthenticatorResponse?, a: Account?, t: String?, o: Bundle?) = null
  override fun hasFeatures(r: AccountAuthenticatorResponse?, a: Account?, f: Array<out String>?) =
    Bundle().apply { putBoolean(android.accounts.AccountManager.KEY_BOOLEAN_RESULT, false) }
}

class AuthenticatorService : Service() {
  private val authenticator by lazy { Authenticator(this) }
  override fun onBind(intent: Intent): IBinder = authenticator.iBinder
}

/**
 * Android asking for a sync of one of the account's services: a change made on the phone
 * (supportsUploading), Sync now in the account's settings, or the app's own button. The sync
 * itself runs as WorkManager work ([SyncWorker]), with its network rule, backoff and time limits;
 * this only hands it over and stays until it is done (DAVx5's SyncAdapterImpl, 2025), so
 * Android shows the account syncing for as long as it is, and the sync settings it keeps (on,
 * off, per service) still decide. Another Apple Account's phone account is refused first.
 */
private class Trigger(context: Context) : AbstractThreadedSyncAdapter(context, true) {
  override fun onPerformSync(a: Account, e: Bundle, authority: String, p: ContentProviderClient, r: SyncResult) {
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
    if (refuseOthers(context, engine, a, authority)) return
    // Android cancelling this sync interrupts the thread, which ends the wait (not the work).
    runCatching {
      runBlocking {
        val name = SyncWork.now(context, a, authority) ?: return@runBlocking
        SyncWork.waitFor(context, name, 10.minutes)
      }
    }
    // Whatever happened is the worker's to retry, with its own backoff: Android's on top
    // would only sync the same service twice.
  }
}

class ContactsSyncService : Service() {
  private val adapter by lazy { Trigger(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

class CalendarSyncService : Service() {
  private val adapter by lazy { Trigger(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

class RemindersSyncService : Service() {
  private val adapter by lazy { Trigger(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

class NotesSyncService : Service() {
  private val adapter by lazy { Trigger(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

class PhotosSyncService : Service() {
  private val adapter by lazy { Trigger(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

/**
 * Calendar: what changed on the phone sent to iCloud first, then iCloud's calendars and a year
 * of events read and written in.
 */
internal fun calendarPass(context: Context, engine: uniffi.quince_mobile.AccountInterface, a: Account, authority: String): Pass = try {
  val (from, to) = CalendarSync.span()
  val tz = java.util.TimeZone.getDefault().id
  val confirmed = Outcome.massRemovalConfirmed(context, authority)
  val (sent, kept) = CalendarSync.send(context.contentResolver, a, confirmed) { engine.calendarSend(it) }
  val window = engine.calendarWindow(from, to, tz)
  val done = CalendarSync.sync(context.contentResolver, a, window, from, to, confirmed, kept)
  val said = buildList {
    add("${done.calendars} calendars, ${window.events.size} events · ${done.added} added, ${done.updated} updated, ${done.removed} removed")
    if (sent.sent > 0) add("${sent.sent} sent to iCloud")
    if (sent.later > 0) add("${sent.later} to send again")
    if (sent.failed > 0) add("${sent.failed} not taken by iCloud: ${sent.reason}")
    if (sent.held > 0) add("${sent.held} changes on this phone held until you confirm")
    if (done.held > 0) add("${done.held} no longer in iCloud, kept on this phone until you confirm")
  }.joinToString(" · ")
  // The sync worked: Apple answered. What it refused stays on this phone, said separately.
  val (here, why) = CalendarSync.onlyHere(context.contentResolver, a)
  Outcome.record(context, authority, true, said, held = done.held + sent.held, onlyHere = here, why = why)
  if (sent.later > 0) Pass.Again else Pass.Done
} catch (x: Exception) {
  if (!x.fromEngine()) throw x
  // Apple busy or unreachable, or the session gone: tried again later, backing off.
  Outcome.record(context, authority, false, x.reason("Calendar couldn't be read."))
  Pass.Again
}

/**
 * Contacts: what changed on the phone sent to iCloud first, then iCloud's address book read and
 * written in, so an edit made here is in iCloud before the book is read, and never written over.
 * The book is read only when iCloud says it changed since the last read written in, or when the
 * phone has rows to write again (ContactsSync.needsRead).
 */
internal fun contactsPass(context: Context, engine: uniffi.quince_mobile.AccountInterface, a: Account, authority: String): Pass = try {
  // One confirmation covers both ways: the changes held from sending and the removals held.
  val confirmed = Outcome.massRemovalConfirmed(context, authority)
  // Labels and cards, through the engine's outbox: one read of the book for all of them.
  val sent = ContactsSync.send(context.contentResolver, a, confirmed) { cards, lists -> engine.contactsSend(cards, lists) }
  val full = confirmed || ContactsSync.needsRead(context.contentResolver, a)
  // Null when iCloud's book hasn't changed since it was last written in: nothing to write.
  val done = when (val read = engine.contactsRead(full)) {
    is uniffi.quince_mobile.BookRead.Unchanged -> null
    is uniffi.quince_mobile.BookRead.Read -> {
      GroupsSync.download(context.contentResolver, a, read.book.lists)
      // A picture at Android's display size; one still too large for a batch (a big PNG kept
      // for its transparency) is asked for again at half the size, never left out.
      val written = ContactsSync.sync(context.contentResolver, a, read.book.people, confirmed) { ref ->
        runCatching {
          engine.contactPhoto(ref, ContactsSync.PHOTO_PX).let {
            if (it.size <= ContactsSync.BATCH_BYTES) it else engine.contactPhoto(ref, ContactsSync.PHOTO_PX / 2u)
          }
        }.getOrNull()
      }
      // Written in: the next pass reads the book again only once iCloud says it changed.
      engine.contactsKept(read.token)
      written
    }
  }
  val said = buildList {
    val people = ContactsSync.cards(context.contentResolver, a)
    add(if (done == null) "$people contacts · unchanged" else "$people contacts · ${done.added} added, ${done.updated} updated, ${done.removed} removed")
    if (sent.sent > 0) add("${sent.sent} sent to iCloud")
    if (sent.later > 0) add("${sent.later} to send again")
    if (sent.failed > 0) add("${sent.failed} not taken by iCloud: ${sent.reason}")
    if (sent.held > 0) add("${sent.held} changes on this phone held until you confirm")
    if ((done?.held ?: 0) > 0) add("${done?.held} no longer in iCloud, kept on this phone until you confirm")
  }.joinToString(" · ")
  // The sync worked: Apple answered. What it refused stays on this phone, said separately.
  val (here, why) = ContactsSync.onlyHere(context.contentResolver, a)
  Outcome.record(context, authority, true, said, held = (done?.held ?: 0) + sent.held, onlyHere = here, why = why)
  if (sent.later > 0) Pass.Again else Pass.Done
} catch (x: Exception) {
  if (!x.fromEngine()) throw x
  Outcome.record(context, authority, false, x.reason("Contacts couldn't be read."))
  Pass.Again
}

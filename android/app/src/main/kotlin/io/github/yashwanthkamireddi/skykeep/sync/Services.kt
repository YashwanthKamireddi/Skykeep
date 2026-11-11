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
 * Android asking for Calendar: what changed on the phone sent to iCloud first, then iCloud's
 * calendars and a year of events read and written in. The only place a calendar sync runs — the
 * app's button asks Android for one — so Apple is asked once, in turn, with Android's backoff
 * after a failure. Android starts one when an event is changed on the phone (supportsUploading).
 */
private class Adapter(context: Context) : AbstractThreadedSyncAdapter(context, true) {
  override fun onPerformSync(a: Account, e: Bundle, authority: String, p: ContentProviderClient, r: SyncResult) {
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
    if (refuseOthers(context, engine, a, authority)) return
    try {
      val (from, to) = CalendarSync.span()
      val tz = java.util.TimeZone.getDefault().id
      val confirmed = Outcome.massRemovalConfirmed(context, authority)
      val sent = CalendarSync.send(context.contentResolver, a, confirmed) { engine.saveEvents(it, from, to, tz) }
      if (sent.later > 0) r.stats.numIoExceptions++
      val window = engine.calendarWindow(from, to, tz)
      val done = CalendarSync.sync(context.contentResolver, a, window, from, to, confirmed)
      r.stats.numInserts += done.added.toLong()
      r.stats.numUpdates += done.updated.toLong()
      r.stats.numDeletes += done.removed.toLong()
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
    } catch (x: uniffi.quince_mobile.EngineException) {
      // Apple busy or unreachable, or the session gone: Android tries again later, backing off.
      r.stats.numIoExceptions++
      Outcome.record(context, authority, false, (x as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "Calendar couldn't be read.")
    }
  }
}

/**
 * Android asking for Contacts: what changed on the phone sent to iCloud first, then iCloud's
 * address book read and written in — so an edit made here is in iCloud before the book is read,
 * and never written over. Android starts one when a contact is changed on the phone
 * (supportsUploading), and hourly.
 */
private class ContactsAdapter(context: Context) : AbstractThreadedSyncAdapter(context, true) {
  override fun onPerformSync(a: Account, e: Bundle, authority: String, p: ContentProviderClient, r: SyncResult) {
    val engine = io.github.yashwanthkamireddi.skykeep.Engine.account(context)
    if (refuseOthers(context, engine, a, authority)) return
    try {
      // One confirmation covers both ways: the changes held from sending and the removals held.
      val confirmed = Outcome.massRemovalConfirmed(context, authority)
      val sent = ContactsSync.send(context.contentResolver, a, confirmed) { engine.saveContacts(it) }
      if (sent.later > 0) r.stats.numIoExceptions++
      // Labels made, renamed or deleted here, and members added or removed, sent after the
      // cards (a new card's id is iCloud's by then); then the book read once: lists, then people.
      GroupsSync.send(context.contentResolver, a, sent.members) { engine.saveLists(it) }
      val book = engine.addressBook()
      GroupsSync.download(context.contentResolver, a, book.lists)
      val people = book.people
      // A picture at Android's display size; one still too large for a batch (a big PNG kept
      // for its transparency) is asked for again at half the size, never left out.
      val done = ContactsSync.sync(context.contentResolver, a, people, confirmed) { ref ->
        runCatching {
          engine.contactPhoto(ref, ContactsSync.PHOTO_PX).let {
            if (it.size <= ContactsSync.BATCH_BYTES) it else engine.contactPhoto(ref, ContactsSync.PHOTO_PX / 2u)
          }
        }.getOrNull()
      }
      r.stats.numInserts += done.added.toLong()
      r.stats.numUpdates += done.updated.toLong()
      r.stats.numDeletes += done.removed.toLong()
      val said = buildList {
        add("${people.size} contacts · ${done.added} added, ${done.updated} updated, ${done.removed} removed")
        if (sent.sent > 0) add("${sent.sent} sent to iCloud")
        if (sent.later > 0) add("${sent.later} to send again")
        if (sent.failed > 0) add("${sent.failed} not taken by iCloud: ${sent.reason}")
        if (sent.held > 0) add("${sent.held} changes on this phone held until you confirm")
        if (done.held > 0) add("${done.held} no longer in iCloud, kept on this phone until you confirm")
      }.joinToString(" · ")
      // The sync worked: Apple answered. What it refused stays on this phone, said separately.
      val (here, why) = ContactsSync.onlyHere(context.contentResolver, a)
      Outcome.record(context, authority, true, said, held = done.held + sent.held, onlyHere = here, why = why)
    } catch (x: uniffi.quince_mobile.EngineException) {
      // Apple busy or unreachable, or the session gone: Android tries again later, backing off.
      r.stats.numIoExceptions++
      Outcome.record(context, authority, false, (x as? uniffi.quince_mobile.EngineException.Failed)?.reason ?: "Contacts couldn't be read.")
    }
  }
}

class ContactsSyncService : Service() {
  private val adapter by lazy { ContactsAdapter(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

class CalendarSyncService : Service() {
  private val adapter by lazy { Adapter(applicationContext) }
  override fun onBind(intent: Intent): IBinder = adapter.syncAdapterBinder
}

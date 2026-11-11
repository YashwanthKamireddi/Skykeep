package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.accounts.AccountManager
import android.content.ContentResolver
import android.content.Context
import android.provider.CalendarContract
import android.provider.ContactsContract

/** The account type Android files Skykeep's contacts and calendars under. */
val ACCOUNT_TYPE = "${io.github.yashwanthkamireddi.skykeep.BuildConfig.APPLICATION_ID}.account"

/** How often Android syncs on its own unless chosen in Settings: hourly, gentle on the battery. */
const val SYNC_EVERY_SECONDS = 60L * 60

/** The interval chosen in Settings, in seconds. */
fun syncEvery(context: Context): Long =
  context.getSharedPreferences("settings", Context.MODE_PRIVATE).getLong("syncEvery", SYNC_EVERY_SECONDS)

/** What the account syncs: Contacts, Calendar, and Reminders and Notes (the engine's own library). */
val SYNCED = listOf(ContactsContract.AUTHORITY, CalendarContract.AUTHORITY, REMINDERS_AUTHORITY, NOTES_AUTHORITY, PHOTOS_AUTHORITY)

/** A new interval: kept, and given to the account's periodic work (SyncWork.every). */
fun setSyncEvery(context: Context, account: Account?, seconds: Long) {
  context.getSharedPreferences("settings", Context.MODE_PRIVATE).edit().putLong("syncEvery", seconds).apply()
  if (account == null) return
  val tag = Work.tagOf(context, account) ?: return
  SyncWork.every(context, account, tag, seconds)
}

/** Where an account keeps the tag of the Apple Account it syncs for (the engine's account tag). */
internal const val ACCOUNT_OWNER = "account"

/** The phone account syncing now, by name: set when the signed-in Apple Account's is made. */
private const val CURRENT = "phoneAccount"

/**
 * The Skykeep account for an Apple Account, created once and made syncable for Contacts,
 * Calendar and Reminders, and marked with the Apple Account's tag. No password is kept in it:
 * the session lives sealed in the Keystore. Run again on every start, so an account made before
 * Reminders synced gains it.
 *
 * Any other Skykeep account on the phone (an earlier Apple Account's, kept when someone else
 * signed in) stops syncing here: its contacts and calendars stay as they are, and nothing of
 * this account's iCloud is ever written into them, nor theirs sent to it.
 */
fun ensureAccount(context: Context, appleId: String, tag: String): Account {
  val account = Account(appleId, ACCOUNT_TYPE)
  val manager = AccountManager.get(context)
  val all = manager.getAccountsByType(ACCOUNT_TYPE)
  if (all.none { it == account }) manager.addAccountExplicitly(account, null, null)
  manager.setUserData(account, ACCOUNT_OWNER, tag)
  context.getSharedPreferences("settings", Context.MODE_PRIVATE).edit().putString(CURRENT, appleId).commit()
  for (other in all) if (other != account) stopSyncing(context, other, tag)
  for (authority in SYNCED) {
    ContentResolver.setIsSyncable(account, authority, 1)
    ContentResolver.setSyncAutomatically(account, authority, true)
  }
  // iCloud sends no push a third-party app can receive, so the account syncs on a schedule:
  // WorkManager batches it with other apps' work, waits for a network, and skips a low battery.
  SyncWork.every(context, account, tag, syncEvery(context))
  // New photos noticed as they are taken, when backup is on.
  PhotoBackup.schedule(context)
  return account
}

/**
 * An account no longer synced on its own nor offered a sync: its rows stay, untouched, and its
 * background work stops (never the signed-in account's, should both carry its tag).
 */
private fun stopSyncing(context: Context, account: Account, current: String) {
  Work.tagOf(context, account)?.takeIf { it != current }?.let { Work.cancelAll(context, it) }
  for (authority in SYNCED) {
    ContentResolver.cancelSync(account, authority)
    ContentResolver.removePeriodicSync(account, authority, android.os.Bundle.EMPTY)
    ContentResolver.setSyncAutomatically(account, authority, false)
    ContentResolver.setIsSyncable(account, authority, 0)
  }
}

/**
 * The phone account of the Apple Account signed in, if it has one here: the one last made by
 * [ensureAccount]; an account from before that was kept is taken when it is the only one.
 */
fun currentAccount(context: Context): Account? {
  val all = AccountManager.get(context).getAccountsByType(ACCOUNT_TYPE)
  val name = context.getSharedPreferences("settings", Context.MODE_PRIVATE).getString(CURRENT, null)
  return if (name != null) all.firstOrNull { it.name == name } else all.singleOrNull()
}

/**
 * Whether a sync Android asks for on `account` is for the Apple Account signed in now: its tag
 * the engine's. Every adapter asks first, so a second account's contacts and calendars are
 * never read with this session, nor this session's changes sent from them. An account made
 * before accounts were tagged is this one's when its name is the Apple ID signed in, and is
 * marked so. With no session, nothing is anyone's.
 */
fun syncsFor(context: Context, engine: uniffi.quince_mobile.AccountInterface, account: Account): Boolean {
  // Android's sync services can be asked about any account; only Skykeep's own are read.
  if (account.type != ACCOUNT_TYPE) return false
  val tag = engine.accountTag() ?: return false
  val manager = AccountManager.get(context)
  val owner = runCatching { manager.getUserData(account, ACCOUNT_OWNER) }.getOrElse { return false }
  if (owner != null) return owner == tag
  if (engine.appleId()?.equals(account.name, ignoreCase = true) != true) return false
  manager.setUserData(account, ACCOUNT_OWNER, tag)
  return true
}

/** The adapters' first step: a sync for another account's phone account is refused and stopped. */
fun refuseOthers(context: Context, engine: uniffi.quince_mobile.AccountInterface, account: Account, authority: String): Boolean {
  if (syncsFor(context, engine, account)) return false
  if (account.type == ACCOUNT_TYPE) ContentResolver.setIsSyncable(account, authority, 0)
  return true
}

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

/** A new interval: kept, and given to Android for everything the account syncs. */
fun setSyncEvery(context: Context, account: Account?, seconds: Long) {
  context.getSharedPreferences("settings", Context.MODE_PRIVATE).edit().putLong("syncEvery", seconds).apply()
  if (account == null) return
  for (authority in SYNCED) {
    ContentResolver.addPeriodicSync(account, authority, android.os.Bundle.EMPTY, seconds)
  }
}

/**
 * The Skykeep account for an Apple Account, created once and made syncable for Contacts,
 * Calendar and Reminders. No password is kept in it: the session lives sealed in the Keystore.
 * Run again on every start, so an account made before Reminders synced gains it.
 */
fun ensureAccount(context: Context, appleId: String): Account {
  val account = Account(appleId, ACCOUNT_TYPE)
  val manager = AccountManager.get(context)
  if (manager.getAccountsByType(ACCOUNT_TYPE).none { it.name == appleId }) {
    manager.addAccountExplicitly(account, null, null)
  }
  for (authority in SYNCED) {
    ContentResolver.setIsSyncable(account, authority, 1)
    ContentResolver.setSyncAutomatically(account, authority, true)
    // iCloud sends no push a third-party app can receive, so Android is asked to sync on a
    // schedule; it batches these with other apps' syncs and skips them without a network.
    ContentResolver.addPeriodicSync(account, authority, android.os.Bundle.EMPTY, syncEvery(context))
  }
  return account
}

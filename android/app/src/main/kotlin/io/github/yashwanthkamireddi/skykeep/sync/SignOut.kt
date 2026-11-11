package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.accounts.AccountManager
import android.content.ContentProviderOperation
import android.content.ContentResolver
import android.content.Context
import android.database.Cursor
import android.net.Uri
import android.provider.CalendarContract
import android.provider.CalendarContract.Calendars
import android.provider.CalendarContract.Events
import android.provider.ContactsContract
import android.provider.ContactsContract.Data
import android.provider.ContactsContract.RawContacts

/**
 * Signing out on this phone, as Android's own account apps do it: the person chooses whether
 * iCloud's contacts and calendars leave the phone with the account, or stay as a copy that is
 * the phone's own (not synced, not lost). iCloud itself is never touched by signing out.
 */
object SignOut {
  /** Changes made on this phone that haven't reached iCloud yet: said before anything goes. */
  fun unsent(cr: ContentResolver, account: Account): Int {
    val contacts = cr.query(
      RawContacts.CONTENT_URI, arrayOf(RawContacts._ID),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=? AND (${RawContacts.DIRTY}=1 OR ${RawContacts.DELETED}=1)",
      arrayOf(account.type, account.name), null,
    )?.use { it.count } ?: 0
    val events = cr.query(
      Events.CONTENT_URI, arrayOf(Events._ID),
      "${Events.ACCOUNT_TYPE}=? AND ${Events.ACCOUNT_NAME}=? AND (${Events.DIRTY}=1 OR ${Events.DELETED}=1)",
      arrayOf(account.type, account.name), null,
    )?.use { it.count } ?: 0
    return contacts + events
  }

  data class Copied(val contacts: Int, val calendars: Int, val events: Int)

  /**
   * The account's contacts copied into the phone's own contacts, and each of its calendars into
   * a calendar of the phone's own ("Work (copy)") with its events: kept when the account goes.
   * Copied whole, every field Android holds, as one batch per card so none is half-copied.
   */
  fun keepCopy(cr: ContentResolver, account: Account): Copied {
    var contacts = 0
    cr.query(
      RawContacts.CONTENT_URI, arrayOf(RawContacts._ID),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=? AND ${RawContacts.DELETED}=0",
      arrayOf(account.type, account.name), null,
    )?.use { raws ->
      while (raws.moveToNext()) {
        val ops = ArrayList<ContentProviderOperation>()
        // The phone's own contacts: no account.
        ops += ContentProviderOperation.newInsert(RawContacts.CONTENT_URI)
          .withValue(RawContacts.ACCOUNT_NAME, null)
          .withValue(RawContacts.ACCOUNT_TYPE, null)
          .build()
        cr.query(Data.CONTENT_URI, null, "${Data.RAW_CONTACT_ID}=?", arrayOf(raws.getLong(0).toString()), null)?.use { rows ->
          while (rows.moveToNext()) {
            val row = ContentProviderOperation.newInsert(Data.CONTENT_URI).withValueBackReference(Data.RAW_CONTACT_ID, 0)
              .withValue(Data.MIMETYPE, rows.getString(rows.getColumnIndexOrThrow(Data.MIMETYPE)))
            for (i in 1..15) copy(rows, "data$i", row)
            ops += row.build()
          }
        }
        cr.applyBatch(ContactsContract.AUTHORITY, ops)
        contacts++
      }
    }

    val local = Account("This phone", CalendarContract.ACCOUNT_TYPE_LOCAL)
    var calendars = 0; var events = 0
    cr.query(
      Calendars.CONTENT_URI, arrayOf(Calendars._ID, Calendars.CALENDAR_DISPLAY_NAME, Calendars.CALENDAR_COLOR),
      "${Calendars.ACCOUNT_TYPE}=? AND ${Calendars.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { cals ->
      while (cals.moveToNext()) {
        val made = cr.insert(
          Calendars.CONTENT_URI.asAdapter(local),
          android.content.ContentValues().apply {
            put(Calendars.ACCOUNT_NAME, local.name)
            put(Calendars.ACCOUNT_TYPE, local.type)
            put(Calendars.NAME, "copy-${cals.getLong(0)}")
            put(Calendars.CALENDAR_DISPLAY_NAME, "${cals.getString(1)} (copy)")
            put(Calendars.CALENDAR_COLOR, cals.getInt(2))
            put(Calendars.CALENDAR_ACCESS_LEVEL, Calendars.CAL_ACCESS_OWNER)
            put(Calendars.OWNER_ACCOUNT, local.name)
            put(Calendars.VISIBLE, 1)
            put(Calendars.SYNC_EVENTS, 1)
          },
        )?.lastPathSegment?.toLongOrNull() ?: continue
        calendars++
        val ops = ArrayList<ContentProviderOperation>()
        cr.query(
          Events.CONTENT_URI,
          arrayOf(Events.TITLE, Events.EVENT_LOCATION, Events.DESCRIPTION, Events.DTSTART, Events.DTEND, Events.ALL_DAY, Events.EVENT_TIMEZONE),
          "${Events.CALENDAR_ID}=? AND ${Events.DELETED}=0", arrayOf(cals.getLong(0).toString()), null,
        )?.use { ev ->
          while (ev.moveToNext()) {
            ops += ContentProviderOperation.newInsert(Events.CONTENT_URI.asAdapter(local))
              .withValue(Events.CALENDAR_ID, made)
              .withValue(Events.TITLE, ev.getString(0))
              .withValue(Events.EVENT_LOCATION, ev.getString(1))
              .withValue(Events.DESCRIPTION, ev.getString(2))
              .withValue(Events.DTSTART, ev.getLong(3))
              .withValue(Events.DTEND, ev.getLong(4))
              .withValue(Events.ALL_DAY, ev.getInt(5))
              .withValue(Events.EVENT_TIMEZONE, ev.getString(6))
              .build()
            if (ops.size >= 300) { cr.applyBatch(CalendarContract.AUTHORITY, ops); events += ops.size; ops.clear() }
          }
        }
        if (ops.isNotEmpty()) { cr.applyBatch(CalendarContract.AUTHORITY, ops); events += ops.size }
      }
    }
    return Copied(contacts, calendars, events)
  }

  /**
   * The account removed from the phone: Android takes its contacts and calendars with it. What
   * the person chose to keep was copied first ([keepCopy]). The sync outcomes, photo backup's
   * settings, the reminder alerts and the cached pictures and copies go too.
   */
  fun removeAccount(context: Context, account: Account): Boolean {
    // Its background work stops first: no sync, backup or send runs for an account leaving.
    Work.tagOf(context, account)?.let { Work.cancelAll(context, it) }
    val gone = AccountManager.get(context).removeAccountExplicitly(account)
    // What belonged to the account's session goes with it, Keep a Copy or not: photo backup's
    // choice and place (the next account never uploads this phone's photos unless it turns
    // backup on itself), and the reminders said or still due.
    for (prefs in listOf("sync-outcome", "photo-backup")) {
      context.getSharedPreferences(prefs, Context.MODE_PRIVATE).edit().clear().commit()
    }
    ReminderAlerts.stop(context)
    // Cached pictures (Hidden ones included), Drive copies and shared copies are the session's
    // too: re-fetched by whoever signs in next, never left readable from this one.
    for (dir in listOf("photos", "drive", "share", "send", "drive-new", "drive-thumbs", "drive-parts")) java.io.File(context.cacheDir, dir).deleteRecursively()
    return gone
  }

  private fun copy(rows: Cursor, column: String, into: ContentProviderOperation.Builder) {
    val i = rows.getColumnIndex(column)
    if (i < 0 || rows.isNull(i)) return
    when (rows.getType(i)) {
      Cursor.FIELD_TYPE_BLOB -> into.withValue(column, rows.getBlob(i))
      Cursor.FIELD_TYPE_INTEGER -> into.withValue(column, rows.getLong(i))
      else -> into.withValue(column, rows.getString(i))
    }
  }

  private fun Uri.asAdapter(a: Account) = buildUpon()
    .appendQueryParameter(CalendarContract.CALLER_IS_SYNCADAPTER, "true")
    .appendQueryParameter(Calendars.ACCOUNT_NAME, a.name)
    .appendQueryParameter(Calendars.ACCOUNT_TYPE, a.type)
    .build()

  /**
   * Remove from Phone: what Skykeep itself keeps for the account deleted with it, beside the
   * engine's library (Account.forgetAccount): the pictures and files fetched (photos, Drive
   * copies, shares), the last syncs' counts and the backup's place. The phone's own contacts
   * and calendars go with the account (removeAccount).
   */
  fun forgetHere(context: Context) {
    for (dir in listOf("photos", "drive", "share", "send", "backup", "drive-upload", "drive-thumbs", "drive-parts")) {
      java.io.File(context.cacheDir, dir).deleteRecursively()
    }
    for (prefs in listOf("sync-outcome", "photo-backup")) {
      context.getSharedPreferences(prefs, Context.MODE_PRIVATE).edit().clear().commit()
    }
  }
}

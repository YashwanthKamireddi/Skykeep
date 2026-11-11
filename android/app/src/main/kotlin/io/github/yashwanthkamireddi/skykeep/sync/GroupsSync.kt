package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.content.ContentProviderOperation
import android.content.ContentResolver
import android.content.ContentValues
import android.net.Uri
import android.provider.ContactsContract
import android.provider.ContactsContract.Groups
import uniffi.quince_mobile.CList
import uniffi.quince_mobile.PhoneList
import uniffi.quince_mobile.SendState
import uniffi.quince_mobile.Settled

/**
 * iCloud's contacts lists as the Contacts app's labels, both ways. A list is a group row (its
 * id in SOURCE_ID); who is in it is a membership row on each contact, written with the contact
 * (ContactsSync.rows) and read back with it (ContactsSync.readPerson).
 *
 * From the phone: a label made, renamed or deleted is that list made, renamed or deleted; a
 * contact added to or taken out of a label is a member added or removed, laid over the list as
 * iCloud has it when sent, so members added on another device meanwhile stay.
 */
object GroupsSync {
  private fun Uri.asAdapter(a: Account) = buildUpon()
    .appendQueryParameter(ContactsContract.CALLER_IS_SYNCADAPTER, "true")
    .appendQueryParameter(Groups.ACCOUNT_NAME, a.name)
    .appendQueryParameter(Groups.ACCOUNT_TYPE, a.type)
    .build()

  private data class Row(val id: Long, val guid: String?, val title: String?, val dirty: Boolean, val deleted: Boolean, val mark: String?)

  private fun rows(cr: ContentResolver, account: Account): List<Row> =
    cr.query(
      Groups.CONTENT_URI.asAdapter(account),
      arrayOf(Groups._ID, Groups.SOURCE_ID, Groups.TITLE, Groups.DIRTY, Groups.DELETED, Groups.SYNC1),
      "${Groups.ACCOUNT_TYPE}=? AND ${Groups.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { c ->
      buildList { while (c.moveToNext()) add(Row(c.getLong(0), c.getString(1), c.getString(2), c.getInt(3) != 0, c.getInt(4) != 0, c.getString(5))) }
    } ?: emptyList()

  /** Each label's row id to its list's id, for reading a contact's memberships. */
  fun guids(cr: ContentResolver, account: Account): Map<Long, String> =
    rows(cr, account).mapNotNull { r -> r.guid?.let { r.id to it } }.toMap()

  /** iCloud's lists written as labels, before the contacts are (their memberships name them). */
  fun download(cr: ContentResolver, account: Account, lists: List<CList>) {
    val have = rows(cr, account).filter { it.guid != null }.associateBy { it.guid!! }
    val ops = ArrayList<ContentProviderOperation>()
    for (l in lists) {
      val row = have[l.guid]
      if (row == null) {
        ops += ContentProviderOperation.newInsert(Groups.CONTENT_URI.asAdapter(account))
          .withValue(Groups.ACCOUNT_NAME, account.name).withValue(Groups.ACCOUNT_TYPE, account.type)
          .withValue(Groups.SOURCE_ID, l.guid).withValue(Groups.TITLE, l.name)
          .withValue(Groups.GROUP_VISIBLE, 1).withValue(Groups.SHOULD_SYNC, 1)
          .build()
      } else if (!row.dirty && !row.deleted && row.title != l.name) {
        ops += ContentProviderOperation.newUpdate(Groups.CONTENT_URI.asAdapter(account))
          .withSelection("${Groups._ID}=?", arrayOf(row.id.toString()))
          .withValue(Groups.TITLE, l.name).build()
      }
    }
    // Lists gone from iCloud: their labels go too, unless changed here and waiting to be sent.
    val listed = lists.map { it.guid }.toSet()
    for ((guid, row) in have) {
      if (guid !in listed && !row.dirty) {
        ops += ContentProviderOperation.newDelete(Groups.CONTENT_URI.asAdapter(account))
          .withSelection("${Groups._ID}=?", arrayOf(row.id.toString())).build()
      }
    }
    if (ops.isNotEmpty()) cr.applyBatch(ContactsContract.AUTHORITY, ops)
  }

  /**
   * The phone's changes to labels, for the engine's outbox: made (given its list's id now, so a
   * send repeated after a lost answer is the same list, and a card added to it can name it),
   * renamed, or deleted. Who is in a label goes with each card (ContactsSync), not here. A label
   * made and deleted here before it was ever sent is simply gone.
   */
  fun changes(cr: ContentResolver, account: Account): List<Pair<Long, PhoneList>> {
    val out = ArrayList<Pair<Long, PhoneList>>()
    for (r in rows(cr, account)) {
      val made = r.mark == MADE_HERE
      when {
        r.deleted && r.guid == null -> remove(cr, account, r.id)
        r.deleted -> out += r.id to PhoneList.Delete(r.guid!!, r.title ?: "")
        r.guid == null && !r.title.isNullOrBlank() -> {
          val guid = java.util.UUID.randomUUID().toString().uppercase()
          cr.update(
            Groups.CONTENT_URI.asAdapter(account),
            ContentValues().apply { put(Groups.SOURCE_ID, guid); put(Groups.SYNC1, MADE_HERE) },
            "${Groups._ID}=?", arrayOf(r.id.toString()),
          )
          out += r.id to PhoneList.Create(guid, r.title)
        }
        // Made here, not yet known to be in iCloud: made again under the same id.
        made && r.guid != null -> out += r.id to PhoneList.Create(r.guid, r.title ?: "")
        r.dirty && r.guid != null -> out += r.id to PhoneList.Rename(r.guid, r.title ?: "")
      }
    }
    return out
  }

  /**
   * Each label handed over, let go of once the engine says it went or was refused: one deleted
   * goes; one made or renamed is no longer waiting, and the next read writes iCloud's own over it
   * (a list iCloud turned down is then gone, its change kept in Settings' Not Sent).
   */
  fun settle(cr: ContentResolver, account: Account, changes: List<Pair<Long, PhoneList>>, settled: Map<String, Settled>) {
    for ((row, change) in changes) {
      val guid = when (change) {
        is PhoneList.Create -> change.guid
        is PhoneList.Rename -> change.guid
        is PhoneList.Delete -> change.guid
      }
      when (settled[guid]?.state) {
        is SendState.Sent, is SendState.Refused ->
          if (change is PhoneList.Delete) remove(cr, account, row)
          else cr.update(
            Groups.CONTENT_URI.asAdapter(account),
            ContentValues().apply { put(Groups.DIRTY, 0); putNull(Groups.SYNC1) },
            "${Groups._ID}=?", arrayOf(row.toString()),
          )
        is SendState.Waiting, null -> {}
      }
    }
  }

  /** A label made on this phone and not yet known to be in iCloud (SYNC1). */
  private const val MADE_HERE = "made-here"

  private fun remove(cr: ContentResolver, account: Account, id: Long) {
    cr.delete(Groups.CONTENT_URI.asAdapter(account), "${Groups._ID}=?", arrayOf(id.toString()))
  }
}

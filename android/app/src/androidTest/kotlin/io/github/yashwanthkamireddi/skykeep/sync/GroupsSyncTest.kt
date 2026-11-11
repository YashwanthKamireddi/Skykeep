package io.github.yashwanthkamireddi.skykeep.sync

import android.Manifest
import android.accounts.Account
import android.accounts.AccountManager
import android.content.ContentResolver
import android.content.ContentValues
import android.provider.CalendarContract
import android.provider.ContactsContract
import android.provider.ContactsContract.CommonDataKinds.GroupMembership
import android.provider.ContactsContract.Data
import android.provider.ContactsContract.Groups
import android.provider.ContactsContract.RawContacts
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.quince_mobile.CList
import uniffi.quince_mobile.PhoneCard
import uniffi.quince_mobile.SendState
import uniffi.quince_mobile.Settled
import uniffi.quince_mobile.Method
import uniffi.quince_mobile.Person

/**
 * iCloud's lists as the Contacts app's labels, both ways, through the real Contacts provider,
 * written as a contacts app writes (no sync adapter flag): the lists arrive as labels with their
 * members; a member added or taken out here is collected as exactly that; a member added in
 * iCloud alone (the card itself unchanged) still arrives. A throwaway account, no iCloud.
 */
@RunWith(AndroidJUnit4::class)
class GroupsSyncTest {
  @get:Rule
  val permissions: GrantPermissionRule = GrantPermissionRule.grant(Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS)

  private val context = InstrumentationRegistry.getInstrumentation().targetContext
  private val cr: ContentResolver = context.contentResolver
  private val account = Account("groups-test@example.invalid", ACCOUNT_TYPE)

  @Before
  fun account() {
    AccountManager.get(context).addAccountExplicitly(account, null, null)
    for (authority in listOf(ContactsContract.AUTHORITY, CalendarContract.AUTHORITY)) ContentResolver.setIsSyncable(account, authority, 0)
  }

  @After
  fun gone() {
    AccountManager.get(context).removeAccountExplicitly(account)
  }

  private fun person(guid: String, first: String, groups: List<String>, etag: String = "e1") = Person(
    guid = guid, etag = etag, firstName = first, middleName = null, lastName = null, company = null, isCompany = false,
    birthday = null, notes = null, methods = listOf(Method("phone", "MOBILE", "+1 555 0$guid")), photo = null,
    prefix = null, suffix = null, nickname = null, phoneticFirstName = null, phoneticLastName = null,
    jobTitle = null, department = null, groups = groups, addresses = null,
  )

  private val lists = listOf(CList("L-FAMILY", "Family", "g1"), CList("L-WORK", "Work", "g1"))

  private fun sync(people: List<Person>) {
    GroupsSync.download(cr, account, lists)
    ContactsSync.sync(cr, account, people, allowMassRemoval = false) { null }
  }

  private fun rawId(guid: String): Long = cr.query(
    RawContacts.CONTENT_URI, arrayOf(RawContacts._ID), "${RawContacts.SOURCE_ID}=? AND ${RawContacts.ACCOUNT_NAME}=?",
    arrayOf(guid, account.name), null,
  )!!.use { it.moveToFirst(); it.getLong(0) }

  private fun labelRow(guid: String): Long = cr.query(
    Groups.CONTENT_URI, arrayOf(Groups._ID), "${Groups.SOURCE_ID}=? AND ${Groups.ACCOUNT_NAME}=?", arrayOf(guid, account.name), null,
  )!!.use { it.moveToFirst(); it.getLong(0) }

  private fun labelsOf(guid: String): Set<Long> = cr.query(
    Data.CONTENT_URI, arrayOf(GroupMembership.GROUP_ROW_ID), "${Data.RAW_CONTACT_ID}=? AND ${Data.MIMETYPE}=?",
    arrayOf(rawId(guid).toString(), GroupMembership.CONTENT_ITEM_TYPE), null,
  )!!.use { c -> buildSet { while (c.moveToNext()) add(c.getLong(0)) } }

  @Test
  fun lists_arrive_as_labels_and_go_back_as_member_edits() {
    sync(listOf(person("1", "Ada", listOf("L-FAMILY")), person("2", "Grace", emptyList())))
    val family = labelRow("L-FAMILY"); val work = labelRow("L-WORK")
    assertEquals("a list's member is in its label", setOf(family), labelsOf("1"))
    assertEquals(emptySet<Long>(), labelsOf("2"))

    // Added to Work in iCloud alone: the card's etag is the same, and the label still arrives.
    sync(listOf(person("1", "Ada", listOf("L-FAMILY")), person("2", "Grace", listOf("L-WORK"))))
    assertEquals("a membership changed in iCloud alone arrives", setOf(work), labelsOf("2"))

    // Here, as a contacts app does it: Ada added to Work, Grace taken out of Work.
    cr.insert(Data.CONTENT_URI, ContentValues().apply {
      put(Data.RAW_CONTACT_ID, rawId("1")); put(Data.MIMETYPE, GroupMembership.CONTENT_ITEM_TYPE); put(GroupMembership.GROUP_ROW_ID, work)
    })
    cr.delete(Data.CONTENT_URI, "${Data.RAW_CONTACT_ID}=? AND ${Data.MIMETYPE}=?", arrayOf(rawId("2").toString(), GroupMembership.CONTENT_ITEM_TYPE))
    var handed: List<PhoneCard> = emptyList()
    ContactsSync.send(cr, account, confirmed = false) { cards, _ -> handed = cards; cards.map { Settled(it.guid, SendState.Sent) } }
    val byGuid = handed.associateBy { it.guid }
    // Each card goes with the labels it is in now and was in when read: the engine sends the
    // difference, laid over each list as iCloud has it.
    assertEquals(setOf("L-FAMILY", "L-WORK"), byGuid["1"]!!.person!!.groups.toSet())
    assertEquals(listOf("L-FAMILY"), byGuid["1"]!!.base!!.groups)
    assertEquals(emptyList<String>(), byGuid["2"]!!.person!!.groups)
    assertEquals(listOf("L-WORK"), byGuid["2"]!!.base!!.groups)
    assertTrue("labels themselves unchanged", family != work)
  }
}

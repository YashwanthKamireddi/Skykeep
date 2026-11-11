package io.github.yashwanthkamireddi.skykeep.sync

import android.Manifest
import android.accounts.Account
import android.accounts.AccountManager
import android.content.ContentResolver
import android.content.ContentValues
import android.provider.CalendarContract
import android.provider.ContactsContract
import android.provider.ContactsContract.CommonDataKinds.Photo
import android.provider.ContactsContract.CommonDataKinds.StructuredName
import android.provider.ContactsContract.Data
import android.provider.ContactsContract.RawContacts
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.quince_mobile.PhoneCard
import uniffi.quince_mobile.SendState
import uniffi.quince_mobile.Settled
import uniffi.quince_mobile.Method
import uniffi.quince_mobile.Person
import uniffi.quince_mobile.PhotoEdit
import uniffi.quince_mobile.PhotoRef

/**
 * A contact's picture both ways, through the real Contacts provider, as a person changes it in a
 * contacts app (written without the sync adapter's flag, as such an app writes): iCloud's picture
 * arrives; an edit to anything else never sends the picture; a picture changed here goes as that
 * picture, and one taken away goes as taken away. Under a throwaway account, no iCloud behind it.
 */
@RunWith(AndroidJUnit4::class)
class PhotoSyncTest {
  @get:Rule
  val permissions: GrantPermissionRule = GrantPermissionRule.grant(
    Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS,
  )

  private val context = InstrumentationRegistry.getInstrumentation().targetContext
  private val cr: ContentResolver = context.contentResolver
  private val account = Account("photo-test@example.invalid", ACCOUNT_TYPE)

  @Before
  fun account() {
    AccountManager.get(context).addAccountExplicitly(account, null, null)
    for (authority in listOf(ContactsContract.AUTHORITY, CalendarContract.AUTHORITY)) {
      ContentResolver.setIsSyncable(account, authority, 0)
    }
  }

  @After
  fun gone() {
    AccountManager.get(context).removeAccountExplicitly(account)
  }

  private fun jpeg(r: Int, g: Int, b: Int): ByteArray {
    val bitmap = android.graphics.Bitmap.createBitmap(300, 300, android.graphics.Bitmap.Config.ARGB_8888)
    bitmap.eraseColor(android.graphics.Color.rgb(r, g, b))
    return java.io.ByteArrayOutputStream().also { bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 90, it) }.toByteArray()
  }

  private val ada = Person(
    guid = "PHOTO-1", etag = "e1", firstName = "Ada", middleName = null, lastName = "Lovelace",
    company = null, isCompany = false, birthday = null, notes = null,
    methods = listOf(Method("phone", "MOBILE", "+1 555 0100")),
    photo = PhotoRef(url = "test:blue", crop = null, key = "blue"),
    prefix = "Countess", suffix = null, nickname = "Ada", phoneticFirstName = null, phoneticLastName = null,
    jobTitle = "Mathematician", department = "Analytical Engine",
    groups = emptyList(), addresses = null,
  )

  /** The phone's changes as the sync would send them, each answered Saved. */
  private fun sent(): List<PhoneCard> {
    var changes: List<PhoneCard> = emptyList()
    ContactsSync.send(cr, account, confirmed = false) { cards, _ -> changes = cards; cards.map { Settled(it.guid, SendState.Sent) } }
    return changes
  }

  private fun rawId(): Long = cr.query(
    RawContacts.CONTENT_URI, arrayOf(RawContacts._ID),
    "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
  )!!.use { it.moveToFirst(); it.getLong(0) }

  /** Written as a contacts app writes: no sync adapter flag, so Android marks the card changed. */
  private fun asAnApp(rawId: Long, mime: String, values: ContentValues) {
    val updated = cr.update(
      Data.CONTENT_URI, values, "${Data.RAW_CONTACT_ID}=? AND ${Data.MIMETYPE}=?", arrayOf(rawId.toString(), mime),
    )
    if (updated == 0) {
      cr.insert(Data.CONTENT_URI, ContentValues(values).apply { put(Data.RAW_CONTACT_ID, rawId); put(Data.MIMETYPE, mime) })
    }
  }

  @Test
  fun a_picture_goes_both_ways_and_only_when_it_changed() {
    val blue = jpeg(40, 90, 220)
    ContactsSync.sync(cr, account, listOf(ada), allowMassRemoval = false) { blue }
    val id = rawId()
    assertTrue("iCloud's picture is on the phone", cr.query(
      Data.CONTENT_URI, arrayOf(Photo.PHOTO_FILE_ID),
      "${Data.RAW_CONTACT_ID}=? AND ${Data.MIMETYPE}=?", arrayOf(id.toString(), Photo.CONTENT_ITEM_TYPE), null,
    )!!.use { it.moveToFirst() && !it.isNull(0) })
    assertTrue("nothing waits to be sent after a sync", sent().isEmpty())

    // Another field edited: the card goes, its picture does not.
    asAnApp(id, StructuredName.CONTENT_ITEM_TYPE, ContentValues().apply { put(StructuredName.GIVEN_NAME, "Augusta Ada") })
    val renamed = sent()
    assertEquals(1, renamed.size)
    assertNull("a name edit never sends the picture", renamed[0].photo)
    // The rest of the name and work, read back from the phone's rows as written: sent as they
    // are, so a name edit never clears a job title or nickname in iCloud.
    val back = renamed[0].person!!
    assertEquals("Mathematician", back.jobTitle)
    assertEquals("Analytical Engine", back.department)
    assertEquals("Ada", back.nickname)
    assertEquals("Countess", back.prefix)

    // Read again from iCloud as it now is (the name edit in it), so the base is current.
    ContactsSync.sync(cr, account, listOf(ada.copy(etag = "e2", firstName = "Augusta Ada")), allowMassRemoval = false) { blue }

    // The picture changed here: sent as that picture.
    asAnApp(rawId(), Photo.CONTENT_ITEM_TYPE, ContentValues().apply { put(Photo.PHOTO, jpeg(220, 40, 40)) })
    val changed = sent()
    assertEquals(1, changed.size)
    val edit = changed[0].photo
    assertTrue("a picture changed here is sent as a picture: $edit", edit is PhotoEdit.Set && edit.image.isNotEmpty())

    // Read back from iCloud with the new picture, then the picture taken away here.
    ContactsSync.sync(cr, account, listOf(ada.copy(etag = "e3", firstName = "Augusta Ada", photo = ada.photo!!.copy(key = "red"))), allowMassRemoval = false) { jpeg(220, 40, 40) }
    assertTrue("nothing waits after reading it back", sent().isEmpty())
    cr.delete(Data.CONTENT_URI, "${Data.RAW_CONTACT_ID}=? AND ${Data.MIMETYPE}=?", arrayOf(rawId().toString(), Photo.CONTENT_ITEM_TYPE))
    val removed = sent()
    assertEquals(1, removed.size)
    assertTrue("a picture taken away here is sent as taken away", removed[0].photo is PhotoEdit.Removed)
  }
}

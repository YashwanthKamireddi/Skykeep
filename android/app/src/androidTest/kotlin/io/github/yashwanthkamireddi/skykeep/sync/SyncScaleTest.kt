package io.github.yashwanthkamireddi.skykeep.sync

import android.Manifest
import android.accounts.Account
import android.accounts.AccountManager
import android.content.ContentResolver
import android.provider.CalendarContract
import android.provider.ContactsContract
import android.provider.ContactsContract.RawContacts
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import java.time.LocalDate
import java.time.ZoneId
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import uniffi.quince_mobile.Cal
import uniffi.quince_mobile.CalendarWindow
import uniffi.quince_mobile.Ev
import uniffi.quince_mobile.Method
import uniffi.quince_mobile.Person

/**
 * The sync at the size the design asks of Phase 1 — 2,000 contacts, 5,000 events — through the
 * real Contacts and Calendar providers, under a throwaway account that never syncs with iCloud
 * (no engine here: iCloud's side is made up). Each count checked, a second sync changing
 * nothing, and the circuit breaker holding a mass removal until it is confirmed.
 */
@RunWith(AndroidJUnit4::class)
class SyncScaleTest {
  @get:Rule
  val permissions: GrantPermissionRule = GrantPermissionRule.grant(
    Manifest.permission.READ_CONTACTS, Manifest.permission.WRITE_CONTACTS,
    Manifest.permission.READ_CALENDAR, Manifest.permission.WRITE_CALENDAR,
  )

  private val context = InstrumentationRegistry.getInstrumentation().targetContext
  private val cr: ContentResolver = context.contentResolver
  private val account = Account("scale-test@example.invalid", ACCOUNT_TYPE)

  @Before
  fun account() {
    AccountManager.get(context).addAccountExplicitly(account, null, null)
    // Never synced by Android: there is no iCloud behind this account.
    for (authority in listOf(ContactsContract.AUTHORITY, CalendarContract.AUTHORITY)) {
      ContentResolver.setIsSyncable(account, authority, 0)
    }
  }

  @After
  fun gone() {
    AccountManager.get(context).removeAccountExplicitly(account)
  }

  private fun person(n: Int, etag: String = "e1") = Person(
    guid = "SCALE-$n", etag = etag, firstName = "Person", middleName = null, lastName = "Number $n",
    company = if (n % 7 == 0) "Company $n" else null, isCompany = false, birthday = null, notes = null,
    methods = listOf(
      Method("phone", "MOBILE", "+1 555 ${n.toString().padStart(4, '0')}"),
      Method("email", null, "person$n@example.invalid"),
    ),
    // Every 50th with a picture, so the picture rows are part of what is timed.
    photo = if (n % 50 == 0) uniffi.quince_mobile.PhotoRef(url = "test:$n", crop = null, key = "k$n") else null,
    prefix = null, suffix = null, nickname = null, phoneticFirstName = null, phoneticLastName = null,
    jobTitle = if (n % 3 == 0) "Engineer" else null, department = null,
    groups = emptyList(), addresses = null,
  )

  /** A picture as the engine hands one over: a small JPEG. */
  private val picture: ByteArray by lazy {
    val bitmap = android.graphics.Bitmap.createBitmap(96, 96, android.graphics.Bitmap.Config.ARGB_8888)
    bitmap.eraseColor(android.graphics.Color.rgb(80, 120, 200))
    java.io.ByteArrayOutputStream().also { bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG, 85, it) }.toByteArray()
  }
  private val photo: (uniffi.quince_mobile.PhotoRef) -> ByteArray? = { picture }

  private fun rows(): Int = cr.query(
    RawContacts.CONTENT_URI, arrayOf(RawContacts._ID),
    "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=? AND ${RawContacts.DELETED}=0",
    arrayOf(account.type, account.name), null,
  )?.use { it.count } ?: -1

  private fun timed(what: String, block: () -> Unit) {
    val started = System.nanoTime()
    block()
    val ms = (System.nanoTime() - started) / 1_000_000
    android.util.Log.i("SkykeepScale", "$what: $ms ms")
  }

  @Test
  fun two_thousand_contacts() {
    val book = (1..2000).map { person(it) }
    timed("2000 contacts, first sync") {
      val r = ContactsSync.sync(cr, account, book, allowMassRemoval = false, photo = photo)
      assertEquals(2000, r.added)
    }
    assertEquals(2000, rows())
    timed("2000 contacts, nothing changed") {
      val r = ContactsSync.sync(cr, account, book, allowMassRemoval = false, photo = photo)
      assertEquals(Triple(0, 0, 2000), Triple(r.added, r.updated, r.unchanged))
    }
    // Ten changed in iCloud: exactly those ten rewritten.
    val edited = book.mapIndexed { i, p -> if (i < 10) person(i + 1, etag = "e2") else p }
    val r1 = ContactsSync.sync(cr, account, edited, allowMassRemoval = false, photo = photo)
    assertEquals(Pair(10, 0), Pair(r1.updated, r1.added))
    // Thirty gone in iCloud: under a tenth of the book, removed.
    val fewer = edited.drop(30)
    val r2 = ContactsSync.sync(cr, account, fewer, allowMassRemoval = false, photo = photo)
    assertEquals(Pair(30, 0), Pair(r2.removed, r2.held))
    assertEquals(1970, rows())
    // A thousand gone at once: held until confirmed, then removed.
    val half = fewer.drop(1000)
    val held = ContactsSync.sync(cr, account, half, allowMassRemoval = false, photo = photo)
    assertEquals(Pair(0, 1000), Pair(held.removed, held.held))
    assertEquals(1970, rows())
    val confirmed = ContactsSync.sync(cr, account, half, allowMassRemoval = true, photo = photo)
    assertEquals(1000, confirmed.removed)
    assertEquals(970, rows())
    // Nothing the sync wrote is marked as changed on the phone (it would be sent back).
    val dirty = cr.query(
      RawContacts.CONTENT_URI, arrayOf(RawContacts._ID),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=? AND ${RawContacts.DIRTY}=1",
      arrayOf(account.type, account.name), null,
    )?.use { it.count }
    assertEquals(0, dirty)
  }

  @Test
  fun five_thousand_events() {
    val (from, to) = CalendarSync.span()
    val zone = ZoneId.systemDefault()
    val first = LocalDate.parse(from).atStartOfDay(zone).toEpochSecond()
    val last = LocalDate.parse(to).atStartOfDay(zone).toEpochSecond()
    val step = (last - first) / 5000
    val events = (0 until 5000).map { n ->
      val starts = first + n * step + 9 * 3600
      Ev(
        guid = "SCALE-EV-$n", calendarGuid = "scale-cal", title = "Event $n", location = null,
        startsAt = starts, endsAt = starts + 1800, allDay = n % 50 == 0, timezone = if (n % 50 == 0) null else "UTC",
        notes = null, url = null, etag = "e1",
      )
    }
    val window = CalendarWindow(listOf(Cal("scale-cal", "Scale test", "#1E88E5", true)), events)
    timed("5000 events, first sync") {
      val r = CalendarSync.sync(cr, account, window, from, to, confirmed = false)
      assertEquals(5000, r.added)
    }
    timed("5000 events, nothing changed") {
      val r = CalendarSync.sync(cr, account, window, from, to, confirmed = false)
      assertEquals(Triple(0, 0, 5000), Triple(r.added, r.updated, r.unchanged))
    }
    val count = cr.query(
      CalendarContract.Events.CONTENT_URI, arrayOf(CalendarContract.Events._ID),
      "${CalendarContract.Events.ACCOUNT_TYPE}=? AND ${CalendarContract.Events.ACCOUNT_NAME}=? AND ${CalendarContract.Events.DELETED}=0",
      arrayOf(account.type, account.name), null,
    )?.use { it.count }
    assertEquals(5000, count)
    assertTrue("every event kept its own day", count == events.size)
  }
}

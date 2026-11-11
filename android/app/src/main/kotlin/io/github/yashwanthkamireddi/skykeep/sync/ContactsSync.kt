package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.content.ContentProviderOperation
import android.content.ContentResolver
import android.net.Uri
import android.provider.ContactsContract
import android.provider.ContactsContract.CommonDataKinds.Email
import android.provider.ContactsContract.CommonDataKinds.Event
import android.provider.ContactsContract.CommonDataKinds.GroupMembership
import android.provider.ContactsContract.CommonDataKinds.Nickname
import android.provider.ContactsContract.CommonDataKinds.Note
import android.provider.ContactsContract.CommonDataKinds.Organization
import android.provider.ContactsContract.CommonDataKinds.Phone
import android.provider.ContactsContract.CommonDataKinds.Photo
import android.provider.ContactsContract.CommonDataKinds.StructuredName
import android.provider.ContactsContract.CommonDataKinds.StructuredPostal
import android.provider.ContactsContract.CommonDataKinds.Website
import android.provider.ContactsContract.Data
import android.provider.ContactsContract.RawContacts
import org.json.JSONArray
import org.json.JSONObject
import uniffi.quince_mobile.Address
import uniffi.quince_mobile.ChangeKind
import uniffi.quince_mobile.Method
import uniffi.quince_mobile.Person
import uniffi.quince_mobile.PhoneCard
import uniffi.quince_mobile.PhoneList
import uniffi.quince_mobile.PhotoEdit
import uniffi.quince_mobile.PhotoRef
import uniffi.quince_mobile.SendState
import uniffi.quince_mobile.Settled

/**
 * iCloud's address book and Android's Contacts, both ways, one card a raw contact under the
 * Skykeep account, found again by Apple's id (SOURCE_ID).
 *
 * Each sync sends first, then reads. Sending: what Android marks changed on the phone (DIRTY),
 * new (no SOURCE_ID yet) or deleted (DELETED) is handed to the engine's outbox — the desktop's
 * own queue and sending code — as the card is now against the card as last synced (its base,
 * kept in SYNC2), so only what changed here is written and a change made on an iPhone meanwhile
 * is kept. A row stays marked changed until the engine says its changes went or were refused.
 * Reading: the book is read only when iCloud says it changed (or a row needs writing again);
 * each card is rewritten only when its etag (SYNC1) changed, and
 * never one still waiting to be sent, so an edit made here is never written over before it
 * reaches iCloud. A card gone from iCloud is removed here — unless iCloud suddenly lists far fewer
 * than this phone has, which is held until the person confirms (the circuit breaker).
 */
object ContactsSync {
  data class Result(
    val added: Int, val updated: Int, val removed: Int, val unchanged: Int,
    /** Cards iCloud no longer lists, kept here until confirmed (see [MASS_REMOVAL]). */
    val held: Int,
  )

  /** `held`: changes the circuit breaker kept back, waiting for the person to confirm. */
  data class Sent(val sent: Int, val failed: Int, val later: Int, val reason: String?, val held: Int)

  /**
   * More than this many cards gone in one sync — and more than a tenth of the book — is held: an
   * account mix-up or a broken answer must never empty a phone's contacts.
   */
  const val MASS_REMOVAL = 25

  private fun Uri.asAdapter(a: Account) = buildUpon()
    .appendQueryParameter(ContactsContract.CALLER_IS_SYNCADAPTER, "true")
    .appendQueryParameter(RawContacts.ACCOUNT_NAME, a.name)
    .appendQueryParameter(RawContacts.ACCOUNT_TYPE, a.type)
    .build()

  /** A card on the phone: its row, etag, whether it waits to be sent, format, and picture's key. */
  private data class Local(
    val id: Long, val etag: String?, val pending: Boolean, val format: String?, val photoKey: String?,
    val groups: Set<String>,
  )

  /**
   * How a card's rows are written here (kept in SYNC3). A row in an older format is written again
   * from iCloud on the next sync, whatever its etag: 6 = addresses part by part (street, city…)
   * and a birthday without a year as Android's --MM-DD; 5 = with its lists (Android's labels);
   * 4 = with the rest of the name and work
   * (prefix, suffix, nickname, phonetic names, job title, department); 3 = with its picture; 2 = Apple's standard
   * labels as Android's own types, a number with no label as Other (1 wrote labels as custom
   * text, none as "phone").
   */
  private const val FORMAT = "6"

  /** The most pictures' bytes in one batch: half of what one binder call can carry. */
  const val BATCH_BYTES = 500_000

  /** A picture's size on the phone: Android's own display size, from which it makes the rest. */
  const val PHOTO_PX = 720u

  /**
   * One sync at a time in the process. Adding the account makes Android start its own sync at
   * once; run beside another, both read an empty book and every card was written twice.
   */
  /** iCloud's book written here; `photo` fetches a card's picture (the engine), null if it can't. */
  fun sync(cr: ContentResolver, account: Account, people: List<Person>, allowMassRemoval: Boolean, photo: (PhotoRef) -> ByteArray?): Result =
    synchronized(this) { download(cr, account, people, allowMassRemoval, photo) }

  /**
   * The phone's changes handed to `send` (the engine's outbox, which sends them as the desktop
   * does): labels made, renamed or deleted, then cards. Each row is let go of once the engine
   * says its changes went or were refused; one still waiting stays marked changed, and is handed
   * over again on the next sync as it is then.
   */
  fun send(
    cr: ContentResolver, account: Account, confirmed: Boolean,
    send: (List<PhoneCard>, List<PhoneList>) -> List<Settled>,
  ): Sent = synchronized(this) { upload(cr, account, confirmed, send) }

  /**
   * Whether the next read must be of the whole book, whatever Apple's tokens say: nothing on the
   * phone yet, or a card the phone holds that must be written again from iCloud — one sent or
   * refused (its etag cleared), or one written in an older format.
   */
  fun needsRead(cr: ContentResolver, account: Account): Boolean {
    var rows = 0
    var stale = false
    cr.query(
      RawContacts.CONTENT_URI.asAdapter(account), arrayOf(RawContacts.SYNC1, RawContacts.SYNC3, RawContacts.DIRTY, RawContacts.DELETED, RawContacts.SOURCE_ID),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { c ->
      while (c.moveToNext()) {
        rows++
        val waiting = c.getInt(2) != 0 || c.getInt(3) != 0
        if (!waiting && c.getString(4) != null && (c.isNull(0) || c.getString(1) != FORMAT)) stale = true
      }
    }
    return rows == 0 || stale
  }

  // --- Sending ---------------------------------------------------------------------------------

  private data class Change(val rawId: Long, val change: PhoneCard)

  private fun upload(
    cr: ContentResolver, account: Account, confirmed: Boolean,
    send: (List<PhoneCard>, List<PhoneList>) -> List<Settled>,
  ): Sent {
    // Labels first: one made here is given its id now, so a card added to it names it.
    val lists = GroupsSync.changes(cr, account)
    val listIds = GroupsSync.guids(cr, account)
    val changes = ArrayList<Change>()
    val dropped = ArrayList<Long>()
    // Marked changed but with no base — the card as last synced — so what changed can't be told
    // (left so by the one-way sync, whose own writes marked every card): never sent. Read again
    // whole from iCloud instead; a deletion of one is undone the same way.
    val unknown = ArrayList<Long>()
    cr.query(
      RawContacts.CONTENT_URI.asAdapter(account),
      arrayOf(RawContacts._ID, RawContacts.SOURCE_ID, RawContacts.SYNC2, RawContacts.DELETED),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=? AND (${RawContacts.DIRTY}=1 OR ${RawContacts.DELETED}=1)",
      arrayOf(account.type, account.name), null,
    )?.use { c ->
      while (c.moveToNext()) {
        val rawId = c.getLong(0)
        val source = c.getString(1)
        val made = c.getString(2) == MADE_HERE
        val base = if (made) null else c.getString(2)?.let(::personFromJson)
        val deleted = c.getInt(3) != 0
        when {
          // Made and deleted here before it was ever sent: nothing for iCloud.
          deleted && source == null -> dropped += rawId
          // Made here and deleted before iCloud said it had it: deleted there too, if it got
          // there (the engine finds no card and takes it as done otherwise).
          deleted && made -> changes += Change(rawId, PhoneCard(ChangeKind.DELETE, source!!, null, null, null))
          // Made here, its first send not answered yet (unreachable, refused for now, or the
          // process gone between choosing its id and sending): handed over again as the same new
          // card. Without its mark it read as a card this phone knew nothing of, and was
          // rewritten from iCloud, which didn't have it: deleted.
          made -> changes += Change(rawId, PhoneCard(ChangeKind.CREATE, source!!, readPerson(cr, rawId, source, null, listIds), null, photoEdit(cr, rawId, null)))
          base == null && source != null -> unknown += rawId
          deleted -> changes += Change(rawId, PhoneCard(ChangeKind.DELETE, source!!, null, base, null))
          source == null -> {
            // New on this phone: its id chosen now and kept, so a send repeated after a lost
            // answer finds the card made rather than making a second.
            val guid = java.util.UUID.randomUUID().toString().uppercase()
            cr.update(
              RawContacts.CONTENT_URI.asAdapter(account),
              android.content.ContentValues().apply { put(RawContacts.SOURCE_ID, guid); put(RawContacts.SYNC2, MADE_HERE) },
              "${RawContacts._ID}=?", arrayOf(rawId.toString()),
            )
            changes += Change(rawId, PhoneCard(ChangeKind.CREATE, guid, readPerson(cr, rawId, guid, null, listIds), null, photoEdit(cr, rawId, null)))
          }
          else -> changes += Change(
            rawId, PhoneCard(ChangeKind.UPDATE, source, readPerson(cr, rawId, source, base, listIds), base, photoEdit(cr, rawId, base)),
          )
        }
      }
    }
    for (id in dropped) removeRow(cr, account, id)
    for (id in unknown) restore(cr, account, id)
    if (changes.isEmpty() && lists.isEmpty()) return Sent(0, 0, 0, null, 0)
    // The circuit breaker, sending: more than MASS_REMOVAL changes and more than a tenth of the
    // book at once is held until the person confirms — a fault must never rewrite a whole book.
    val book = count(cr, account)
    if (!confirmed && changes.size > MASS_REMOVAL && changes.size * 10 > book) {
      return Sent(0, 0, 0, null, changes.size)
    }
    val states = send(changes.map { it.change }, lists.map { it.second }).associateBy { it.guid }
    GroupsSync.settle(cr, account, lists, states)
    var sent = 0; var failed = 0; var later = 0; var reason: String? = null
    for (c in changes) {
      when (val s = states[c.change.guid]?.state) {
        is SendState.Sent -> {
          sent++
          if (c.change.kind == ChangeKind.DELETE) removeRow(cr, account, c.rawId) else settled(cr, account, c.rawId)
        }
        // Kept by the engine, in order, and handed over again next time: the row stays changed.
        is SendState.Waiting, null -> later++
        // Kept by the engine for Try Again or Discard (Settings, Not Sent); here the row is let
        // go of, and the next read shows iCloud's own.
        is SendState.Refused -> { reason = s.reason; failed++; settled(cr, account, c.rawId, kept = s.reason) }
      }
    }
    return Sent(sent, failed, later, reason, 0)
  }

  /** The labels a card is in on the phone, by their lists' ids. */
  private fun memberGuids(cr: ContentResolver, rawId: Long, listIds: Map<Long, String>): List<String> =
    cr.query(
      Data.CONTENT_URI, arrayOf(GroupMembership.GROUP_ROW_ID),
      "${Data.RAW_CONTACT_ID}=? AND ${Data.MIMETYPE}=?", arrayOf(rawId.toString(), GroupMembership.CONTENT_ITEM_TYPE), null,
    )?.use { c -> buildList { while (c.moveToNext()) if (!c.isNull(0)) listIds[c.getLong(0)]?.let(::add) } }?.distinct() ?: emptyList()

  private fun count(cr: ContentResolver, account: Account): Int =
    cr.query(
      RawContacts.CONTENT_URI.asAdapter(account), arrayOf(RawContacts._ID),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { it.count } ?: 0

  /** The account's cards on the phone (not those deleted here and waiting to go). */
  fun cards(cr: ContentResolver, account: Account): Int =
    cr.query(
      RawContacts.CONTENT_URI.asAdapter(account), arrayOf(RawContacts._ID),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=? AND ${RawContacts.DELETED}=0", arrayOf(account.type, account.name), null,
    )?.use { it.count } ?: 0

  /** Not sent, not waiting: undeleted, and rewritten from iCloud whole on the reading pass. */
  private fun restore(cr: ContentResolver, account: Account, rawId: Long) {
    cr.update(
      RawContacts.CONTENT_URI.asAdapter(account),
      android.content.ContentValues().apply {
        put(RawContacts.DIRTY, 0)
        put(RawContacts.DELETED, 0)
        putNull(RawContacts.SYNC1)
      },
      "${RawContacts._ID}=?", arrayOf(rawId.toString()),
    )
  }

  /**
   * Sent, or refused: no longer waiting, and read again from iCloud whole on the reading pass. A
   * refusal's reason is kept (SYNC4) for as long as iCloud doesn't have the card, so the app says
   * so every time rather than once.
   */
  private fun settled(cr: ContentResolver, account: Account, rawId: Long, kept: String? = null) {
    cr.update(
      RawContacts.CONTENT_URI.asAdapter(account),
      android.content.ContentValues().apply {
        put(RawContacts.DIRTY, 0)
        putNull(RawContacts.SYNC1)
        if (kept != null) put(RawContacts.SYNC4, kept) else putNull(RawContacts.SYNC4)
      },
      "${RawContacts._ID}=?", arrayOf(rawId.toString()),
    )
  }

  /** Cards iCloud couldn't take that stay on this phone alone, and the first one's reason. */
  fun onlyHere(cr: ContentResolver, account: Account): Pair<Int, String?> =
    cr.query(
      RawContacts.CONTENT_URI.asAdapter(account), arrayOf(RawContacts.SYNC4),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=? AND ${RawContacts.DELETED}=0 AND ${RawContacts.SYNC4} IS NOT NULL",
      arrayOf(account.type, account.name), null,
    )?.use { c -> c.count to (if (c.moveToFirst()) c.getString(0) else null) } ?: (0 to null)

  private fun removeRow(cr: ContentResolver, account: Account, rawId: Long) {
    cr.delete(RawContacts.CONTENT_URI.asAdapter(account), "${RawContacts._ID}=?", arrayOf(rawId.toString()))
  }

  /**
   * A raw contact as it is on the phone now, in the engine's shape. Labels the phone shows plainly
   * ("Mobile") go back as Apple wrote them ("_$!<Mobile>!$_") when the base had one of that kind,
   * else as Apple's own names where they are one, so an edit to one number never turns the others'
   * labels into custom text in iCloud.
   */
  /**
   * The card's picture as changed on the phone, or null where it is as iCloud has it. The row the
   * sync wrote carries iCloud's key (SYNC1) and the file Android made of it (SYNC2, see
   * [markPictures]): a picture row without the key was put there by the person, and one whose
   * file is no longer that file was edited by them. No picture where the base had one: taken away.
   */
  private fun photoEdit(cr: ContentResolver, rawId: Long, base: Person?): PhotoEdit? {
    var row: Triple<String?, String?, Long?>? = null
    var hasPicture = false
    cr.query(
      Data.CONTENT_URI, arrayOf(Data.SYNC1, Data.SYNC2, Photo.PHOTO_FILE_ID, Photo.PHOTO),
      "${Data.RAW_CONTACT_ID}=? AND ${Data.MIMETYPE}=?", arrayOf(rawId.toString(), Photo.CONTENT_ITEM_TYPE), null,
    )?.use { c ->
      if (c.moveToFirst()) {
        row = Triple(c.getString(0), c.getString(1), if (c.isNull(2)) null else c.getLong(2))
        hasPicture = !c.isNull(2) || !c.isNull(3)
      }
    }
    if (!hasPicture) return if (base?.photo != null) PhotoEdit.Removed else null
    val (key, file, now) = row!!
    val ours = key != null && key == base?.photo?.key && file != null && file == now?.toString()
    if (ours) return null
    return picture(cr, rawId)?.let { PhotoEdit.Set(it) }
  }

  /**
   * Each picture the sync wrote, marked with the file Android made of it (SYNC2), once Android has
   * made it: a picture the person changes gets a new file, which is how the change is found.
   * Android's own edit count (DATA_VERSION) can't tell it: its processing of the sync's own
   * picture counts as edits (2 on the emulator, 10 Nov).
   */
  private fun markPictures(cr: ContentResolver, account: Account) {
    val unmarked = ArrayList<Pair<Long, Long>>()
    cr.query(
      Data.CONTENT_URI.asAdapter(account), arrayOf(Data._ID, Photo.PHOTO_FILE_ID),
      "${Data.MIMETYPE}=? AND ${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=? AND ${Data.SYNC1} IS NOT NULL AND ${Data.SYNC2} IS NULL AND ${Photo.PHOTO_FILE_ID} IS NOT NULL",
      arrayOf(Photo.CONTENT_ITEM_TYPE, account.type, account.name), null,
    )?.use { c -> while (c.moveToNext()) unmarked += c.getLong(0) to c.getLong(1) }
    if (unmarked.isEmpty()) return
    val ops = unmarked.map { (id, file) ->
      ContentProviderOperation.newUpdate(Data.CONTENT_URI.asAdapter(account))
        .withSelection("${Data._ID}=?", arrayOf(id.toString()))
        .withValue(Data.SYNC2, file.toString())
        .build()
    }
    ops.chunked(300).forEach { cr.applyBatch(ContactsContract.AUTHORITY, ArrayList(it)) }
  }

  /**
   * The picture of the card with Apple's id `guid`, as this phone already holds it, if it has
   * one: the person's own card's is the account's picture. Read from the phone, never asked of
   * Apple.
   */
  fun pictureOf(cr: ContentResolver, account: Account, guid: String): ByteArray? {
    val rawId = cr.query(
      RawContacts.CONTENT_URI.asAdapter(account), arrayOf(RawContacts._ID),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=? AND ${RawContacts.SOURCE_ID}=? AND ${RawContacts.DELETED}=0",
      arrayOf(account.type, account.name, guid), null,
    )?.use { if (it.moveToFirst()) it.getLong(0) else null } ?: return null
    return picture(cr, rawId)
  }

  /** The card's picture at its full size (Android's display photo), else the small one. */
  private fun picture(cr: ContentResolver, rawId: Long): ByteArray? {
    val display = Uri.withAppendedPath(
      android.content.ContentUris.withAppendedId(RawContacts.CONTENT_URI, rawId), RawContacts.DisplayPhoto.CONTENT_DIRECTORY,
    )
    runCatching { cr.openAssetFileDescriptor(display, "r")?.use { it.createInputStream().readBytes() } }.getOrNull()
      ?.takeIf { it.isNotEmpty() }?.let { return it }
    return cr.query(
      Data.CONTENT_URI, arrayOf(Photo.PHOTO),
      "${Data.RAW_CONTACT_ID}=? AND ${Data.MIMETYPE}=?", arrayOf(rawId.toString(), Photo.CONTENT_ITEM_TYPE), null,
    )?.use { c -> if (c.moveToFirst()) c.getBlob(0) else null }
  }

  private fun readPerson(cr: ContentResolver, rawId: Long, guid: String, base: Person?, listIds: Map<Long, String>): Person {
    var first: String? = null; var middle: String? = null; var last: String? = null
    var company: String? = null; var notes: String? = null; var birthday: String? = null
    var prefix: String? = null; var suffix: String? = null; var nickname: String? = null
    var phoneticFirst: String? = null; var phoneticLast: String? = null
    var jobTitle: String? = null; var department: String? = null
    val methods = ArrayList<Method>()
    val addresses = readAddresses(cr, rawId, base)
    cr.query(
      Data.CONTENT_URI,
      arrayOf(Data.MIMETYPE, Data.DATA1, Data.DATA2, Data.DATA3, Data.DATA5, Data.DATA4, Data.DATA6, Data.DATA7, Data.DATA9),
      "${Data.RAW_CONTACT_ID}=?", arrayOf(rawId.toString()), "${Data._ID} ASC",
    )?.use { c ->
      while (c.moveToNext()) {
        val mime = c.getString(0)
        fun text(i: Int) = c.getString(i)?.takeIf { it.isNotBlank() }
        when (mime) {
          // GIVEN_NAME is DATA2, FAMILY_NAME DATA3, MIDDLE_NAME DATA5 (columns 2, 3, 4 here).
          // PREFIX DATA4, SUFFIX DATA6, PHONETIC_GIVEN_NAME DATA7, PHONETIC_FAMILY_NAME DATA9.
          StructuredName.CONTENT_ITEM_TYPE -> {
            first = text(2); last = text(3); middle = text(4)
            prefix = text(5); suffix = text(6); phoneticFirst = text(7); phoneticLast = text(8)
          }
          // COMPANY DATA1, TITLE DATA4, DEPARTMENT DATA5.
          Organization.CONTENT_ITEM_TYPE -> { company = text(1); jobTitle = text(5); department = text(4) }
          Nickname.CONTENT_ITEM_TYPE -> nickname = text(1)
          Note.CONTENT_ITEM_TYPE -> notes = text(1)
          Event.CONTENT_ITEM_TYPE -> if (c.getInt(2) == Event.TYPE_BIRTHDAY) birthday = text(1)
          // Android's own type (Mobile, Home, Work…) or, for TYPE_CUSTOM, the label typed.
          // Addresses are read part by part (readAddresses), not as a line here.
          Phone.CONTENT_ITEM_TYPE, Email.CONTENT_ITEM_TYPE, Website.CONTENT_ITEM_TYPE ->
            text(1)?.let { value ->
              val kind = KIND.getValue(mime)
              val type = c.getInt(2)
              val label = if (type == CUSTOM) text(3) else TYPES[mime]?.entries?.firstOrNull { it.value == type }?.key
              methods += Method(kind, appleLabel(kind, label, value, base), value)
            }
        }
      }
    }
    return Person(
      guid = guid, etag = base?.etag, firstName = first, middleName = middle, lastName = last,
      company = company, isCompany = base?.isCompany ?: false, birthday = birthday, notes = notes,
      methods = methods, photo = base?.photo,
      prefix = prefix, suffix = suffix, nickname = nickname,
      phoneticFirstName = phoneticFirst, phoneticLastName = phoneticLast,
      jobTitle = jobTitle, department = department, groups = memberGuids(cr, rawId, listIds),
      addresses = addresses,
    )
  }

  /**
   * A card's addresses as the phone holds them, part by part, in the order they were written
   * (so each lines up with the base's in the same place). A PO box, which iCloud has no part for,
   * goes on the street's last line. The label goes back as the base spelled it when the phone
   * shows the same one, as [appleLabel] does for numbers.
   */
  private fun readAddresses(cr: ContentResolver, rawId: Long, base: Person?): List<Address> {
    val out = ArrayList<Address>()
    cr.query(
      Data.CONTENT_URI,
      arrayOf(
        StructuredPostal.STREET, StructuredPostal.POBOX, StructuredPostal.NEIGHBORHOOD, StructuredPostal.CITY,
        StructuredPostal.REGION, StructuredPostal.POSTCODE, StructuredPostal.COUNTRY, StructuredPostal.TYPE,
        StructuredPostal.LABEL, StructuredPostal.FORMATTED_ADDRESS,
      ),
      "${Data.RAW_CONTACT_ID}=? AND ${Data.MIMETYPE}=?", arrayOf(rawId.toString(), StructuredPostal.CONTENT_ITEM_TYPE), "${Data._ID} ASC",
    )?.use { c ->
      while (c.moveToNext()) {
        fun text(i: Int) = c.getString(i)?.trim()?.takeIf { it.isNotEmpty() }
        val street = listOfNotNull(text(0), text(1)).joinToString("\n").ifEmpty { null }
        val type = c.getInt(7)
        val shownLabel = if (type == CUSTOM) text(8) else TYPES[StructuredPostal.CONTENT_ITEM_TYPE]?.entries?.firstOrNull { it.value == type }?.key
        val was = base?.addresses?.getOrNull(out.size)
        val label = if (was != null && shown(was.label) == (key(shownLabel) ?: "OTHER")) was.label else shownLabel
        val a = Address(
          label = label, street = street, subLocality = text(2), city = text(3),
          subAdministrativeArea = null, state = text(4), postalCode = text(5), country = text(6),
          countryCode = null, formatted = text(9) ?: "",
        )
        if (listOf(a.street, a.subLocality, a.city, a.state, a.postalCode, a.country).any { it != null }) out += a
      }
    }
    return out
  }

  /**
   * A card made on this phone and not yet known to be in iCloud: kept in its base's place
   * (SYNC2), as a card has no base until iCloud has it. The reading pass writes the real base
   * over it once the card comes back from iCloud.
   */
  private const val MADE_HERE = "made-here"

  /** Android's TYPE_CUSTOM, the same number for every kind. */
  private const val CUSTOM = 0

  private val KIND = mapOf(
    Phone.CONTENT_ITEM_TYPE to "phone", Email.CONTENT_ITEM_TYPE to "email", Website.CONTENT_ITEM_TYPE to "url",
  )

  /**
   * Apple's standard labels, by their key (upper case, no spaces, unwrapped), as Android's own
   * types: shown in the phone's language ("Mobile", "Mobil"…) rather than as custom text.
   * Apple writes them either way — "MOBILE" or "_$!<Mobile>!$_" — on one card.
   */
  private val TYPES: Map<String, Map<String, Int>> = mapOf(
    Phone.CONTENT_ITEM_TYPE to mapOf(
      "MOBILE" to Phone.TYPE_MOBILE, "HOME" to Phone.TYPE_HOME, "WORK" to Phone.TYPE_WORK,
      "MAIN" to Phone.TYPE_MAIN, "HOMEFAX" to Phone.TYPE_FAX_HOME, "WORKFAX" to Phone.TYPE_FAX_WORK,
      "OTHERFAX" to Phone.TYPE_OTHER_FAX, "PAGER" to Phone.TYPE_PAGER, "OTHER" to Phone.TYPE_OTHER,
    ),
    Email.CONTENT_ITEM_TYPE to mapOf("HOME" to Email.TYPE_HOME, "WORK" to Email.TYPE_WORK, "OTHER" to Email.TYPE_OTHER),
    Website.CONTENT_ITEM_TYPE to mapOf(
      "HOMEPAGE" to Website.TYPE_HOMEPAGE, "HOME" to Website.TYPE_HOME, "WORK" to Website.TYPE_WORK,
      "OTHER" to Website.TYPE_OTHER,
    ),
    StructuredPostal.CONTENT_ITEM_TYPE to mapOf(
      "HOME" to StructuredPostal.TYPE_HOME, "WORK" to StructuredPostal.TYPE_WORK, "OTHER" to StructuredPostal.TYPE_OTHER,
    ),
  )

  /** A label's key: "_$!<Home FAX>!$_" and "HOMEFAX" are the same label. */
  private fun key(label: String?) = label?.removePrefix("_\$!<")?.removeSuffix(">!\$_")?.replace(" ", "")?.uppercase()

  /** A label as shown: the wrapping taken off. */
  private fun plain(label: String?) = label?.removePrefix("_\$!<")?.removeSuffix(">!\$_")

  /** How a label of Apple's shows on the phone, as a key: none shows as Other. */
  private fun shown(label: String?) = key(label) ?: "OTHER"

  /**
   * The label to send for one read off the phone (a type's key, or custom text). The same number
   * or address showing the same label as when synced goes back with exactly the label it had —
   * none included (most numbers have none; sent back as "Other" they would all have changed).
   * Otherwise the base's own spelling of that label, else the key, as Apple writes its own.
   */
  private fun appleLabel(kind: String, label: String?, value: String, base: Person?): String? {
    val methods = base?.methods.orEmpty().filter { it.kind == kind }
    methods.firstOrNull { it.value == value && shown(it.label) == key(label) }?.let { return it.label }
    if (label == null) return null
    methods.firstOrNull { it.label != null && key(it.label) == key(label) }?.let { return it.label }
    return label
  }

  // --- Reading ---------------------------------------------------------------------------------

  private fun download(
    cr: ContentResolver, account: Account, people: List<Person>, allowMassRemoval: Boolean,
    photo: (PhotoRef) -> ByteArray?,
  ): Result {
    val local = HashMap<String, Local>()
    // A card found twice (left by an earlier fault) keeps one copy; the others are removed.
    val extra = ArrayList<Long>()
    cr.query(
      RawContacts.CONTENT_URI.asAdapter(account),
      arrayOf(RawContacts._ID, RawContacts.SOURCE_ID, RawContacts.SYNC1, RawContacts.DIRTY, RawContacts.DELETED, RawContacts.SYNC3, RawContacts.SYNC2),
      "${RawContacts.ACCOUNT_TYPE}=? AND ${RawContacts.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { c ->
      while (c.moveToNext()) {
        val source = c.getString(1) ?: continue
        val pending = c.getInt(3) != 0 || c.getInt(4) != 0
        if (source in local) { if (!pending) extra += c.getLong(0); continue }
        val base = c.getString(6)?.let(::personFromJson)
        local[source] = Local(c.getLong(0), c.getString(2), pending, c.getString(5), base?.photo?.key, base?.groups?.toSet() ?: emptySet())
      }
    }
    var added = 0; var updated = 0; var unchanged = 0
    val ops = ArrayList<ContentProviderOperation>()
    // Pictures' bytes in the batch: a batch crosses to Android's Contacts in one binder call,
    // which takes about 1 MB, and 300 cards' pictures made one of 4.5 MB that crashed the sync
    // (10 Nov). So a batch is cut by size as well as by count.
    var bytes = 0
    fun flush() { if (ops.isNotEmpty()) { cr.applyBatch(ContactsContract.AUTHORITY, ops); ops.clear() }; bytes = 0 }

    for (p in people) {
      val have = local.remove(p.guid)
      // Changed here and not yet in iCloud: left as it is, sent on the next sync.
      if (have != null && have.pending) continue
      // A card's lists are changed on the lists, not the card (its etag stays), so they are
      // compared too: without it, a contact added to a list on an iPhone never showed here.
      if (have != null && have.etag != null && have.etag == p.etag && have.format == FORMAT &&
        have.photoKey == p.photo?.key && have.groups == p.groups.toSet()
      ) {
        unchanged++; continue
      }
      // The picture: fetched only when it is not the one the phone has. A picture that can't be
      // had now leaves the card without one, its key unrecorded, so the next sync tries again.
      val keepPicture = have != null && have.format == FORMAT && have.photoKey != null && have.photoKey == p.photo?.key
      val picture = if (keepPicture) null else p.photo?.let(photo)
      val written = if (keepPicture || picture != null) p else p.copy(photo = null)
      if (ops.size > 300 || bytes + (picture?.size ?: 0) > BATCH_BYTES) flush()
      bytes += picture?.size ?: 0
      val start = ops.size
      if (have == null) {
        ops += ContentProviderOperation.newInsert(RawContacts.CONTENT_URI.asAdapter(account))
          .withValue(RawContacts.ACCOUNT_NAME, account.name)
          .withValue(RawContacts.ACCOUNT_TYPE, account.type)
          .withValue(RawContacts.SOURCE_ID, p.guid)
          .withValue(RawContacts.SYNC1, p.etag)
          .withValue(RawContacts.SYNC2, personJson(written))
          .withValue(RawContacts.SYNC4, null)
          .withValue(RawContacts.SYNC3, FORMAT)
          .build()
        rows(p, account, picture).forEach { ops += it.withValueBackReference(Data.RAW_CONTACT_ID, start).build() }
        added++
      } else {
        ops += ContentProviderOperation.newUpdate(RawContacts.CONTENT_URI.asAdapter(account))
          .withSelection("${RawContacts._ID}=?", arrayOf(have.id.toString()))
          .withValue(RawContacts.SYNC1, p.etag)
          .withValue(RawContacts.SYNC2, personJson(written))
          .withValue(RawContacts.SYNC4, null)
          .withValue(RawContacts.SYNC3, FORMAT)
          .build()
        // Every row rewritten from iCloud's card, but a picture that hasn't changed stays as it
        // is: fetched again on every edit to a card, a book of faces would cost a download each.
        ops += ContentProviderOperation.newDelete(Data.CONTENT_URI.asAdapter(account))
          .withSelection(
            if (keepPicture) "${Data.RAW_CONTACT_ID}=? AND ${Data.MIMETYPE}!=?" else "${Data.RAW_CONTACT_ID}=?",
            if (keepPicture) arrayOf(have.id.toString(), Photo.CONTENT_ITEM_TYPE) else arrayOf(have.id.toString()),
          )
          .build()
        rows(p, account, picture).forEach { ops += it.withValue(Data.RAW_CONTACT_ID, have.id).build() }
        updated++
      }
    }
    // Cards no longer in iCloud (never one waiting to be sent), and second copies.
    val gone = local.values.filter { !it.pending }.map { it.id }
    val massive = gone.size > MASS_REMOVAL && gone.size * 10 > (people.size + gone.size)
    val removing = if (massive && !allowMassRemoval) emptyList() else gone
    for (id in removing + extra) {
      if (ops.size > 300) flush()
      ops += ContentProviderOperation.newDelete(RawContacts.CONTENT_URI.asAdapter(account))
        .withSelection("${RawContacts._ID}=?", arrayOf(id.toString()))
        .build()
    }
    flush()
    markPictures(cr, account)
    return Result(added, updated, removing.size + extra.size, unchanged, gone.size - removing.size)
  }

  /** A card's data rows, each still to be pointed at its raw contact. */
  private fun rows(p: Person, account: Account, picture: ByteArray?): List<ContentProviderOperation.Builder> {
    val out = ArrayList<ContentProviderOperation.Builder>()
    // As the sync adapter: written by Android's own Contacts address, every card the sync wrote
    // was marked changed on the phone (DIRTY), and a two-way sync would have sent them all back.
    fun row(mime: String) = ContentProviderOperation.newInsert(Data.CONTENT_URI.asAdapter(account)).withValue(Data.MIMETYPE, mime)
    out += row(StructuredName.CONTENT_ITEM_TYPE)
      .withValue(StructuredName.GIVEN_NAME, p.firstName)
      .withValue(StructuredName.MIDDLE_NAME, p.middleName)
      .withValue(StructuredName.FAMILY_NAME, p.lastName)
      .withValue(StructuredName.PREFIX, p.prefix)
      .withValue(StructuredName.SUFFIX, p.suffix)
      .withValue(StructuredName.PHONETIC_GIVEN_NAME, p.phoneticFirstName)
      .withValue(StructuredName.PHONETIC_FAMILY_NAME, p.phoneticLastName)
      .apply { if (p.isCompany || (p.firstName == null && p.lastName == null)) withValue(StructuredName.DISPLAY_NAME, p.company) }
    // The picture whole: Android keeps it as the card's large photo and makes the small one for
    // lists itself. Its key (SYNC1 on the row) says which of iCloud's pictures it is; the file
    // Android makes of it is marked after (markPictures), which is how a change here is found.
    if (picture != null && p.photo != null) {
      out += row(Photo.CONTENT_ITEM_TYPE).withValue(Photo.PHOTO, picture)
        .withValue(Data.IS_PRIMARY, 1).withValue(Data.IS_SUPER_PRIMARY, 1)
        .withValue(Data.SYNC1, p.photo!!.key)
    }
    if (p.company != null || p.jobTitle != null || p.department != null) {
      out += row(Organization.CONTENT_ITEM_TYPE)
        .withValue(Organization.COMPANY, p.company)
        .withValue(Organization.TITLE, p.jobTitle)
        .withValue(Organization.DEPARTMENT, p.department)
    }
    p.nickname?.let { out += row(Nickname.CONTENT_ITEM_TYPE).withValue(Nickname.NAME, it) }
    // Its lists, by the list's id: Android finds the label (GroupsSync wrote it first).
    for (g in p.groups) out += row(GroupMembership.CONTENT_ITEM_TYPE).withValue(GroupMembership.GROUP_SOURCE_ID, g)
    p.notes?.let { out += row(Note.CONTENT_ITEM_TYPE).withValue(Note.NOTE, it) }
    p.birthday?.let {
      out += row(Event.CONTENT_ITEM_TYPE).withValue(Event.TYPE, Event.TYPE_BIRTHDAY).withValue(Event.START_DATE, it)
    }
    for (m in p.methods) {
      val mime = KIND.entries.firstOrNull { it.value == m.kind }?.key ?: continue
      // No label shows as Other (and goes back as none: appleLabel).
      val type = TYPES[mime]?.get(shown(m.label))
      val valueColumn = when (mime) {
        Phone.CONTENT_ITEM_TYPE -> Phone.NUMBER
        Email.CONTENT_ITEM_TYPE -> Email.ADDRESS
        else -> Website.URL
      }
      // Apple's standard labels as Android's own types; anything else as its own text.
      out += row(mime).withValue(valueColumn, m.value).apply {
        if (type != null) withValue(Data.DATA2, type)
        else withValue(Data.DATA2, CUSTOM).withValue(Data.DATA3, plain(m.label) ?: m.kind)
      }
    }
    // Addresses part by part, so a part changed here goes back to iCloud as that part alone
    // (readAddresses), with the whole as Apple lays it out for the line Android shows.
    for (a in p.addresses.orEmpty()) {
      val type = TYPES[StructuredPostal.CONTENT_ITEM_TYPE]?.get(shown(a.label))
      out += row(StructuredPostal.CONTENT_ITEM_TYPE)
        .withValue(StructuredPostal.STREET, a.street)
        .withValue(StructuredPostal.NEIGHBORHOOD, a.subLocality)
        .withValue(StructuredPostal.CITY, a.city)
        .withValue(StructuredPostal.REGION, a.state)
        .withValue(StructuredPostal.POSTCODE, a.postalCode)
        .withValue(StructuredPostal.COUNTRY, a.country)
        .withValue(StructuredPostal.FORMATTED_ADDRESS, a.formatted.ifBlank { null })
        .apply {
          if (type != null) withValue(Data.DATA2, type)
          else withValue(Data.DATA2, CUSTOM).withValue(Data.DATA3, plain(a.label) ?: "address")
        }
    }
    return out
  }

  // --- The base, as kept in SYNC2 -------------------------------------------------------------

  fun personJson(p: Person): String = JSONObject().apply {
    put("guid", p.guid); put("etag", p.etag); put("first", p.firstName); put("middle", p.middleName)
    put("last", p.lastName); put("company", p.company); put("isCompany", p.isCompany)
    put("birthday", p.birthday); put("notes", p.notes); put("photoKey", p.photo?.key)
    put("prefix", p.prefix); put("suffix", p.suffix); put("nickname", p.nickname)
    put("phoneticFirst", p.phoneticFirstName); put("phoneticLast", p.phoneticLastName)
    put("jobTitle", p.jobTitle); put("department", p.department)
    put("groups", JSONArray().apply { p.groups.forEach { put(it) } })
    put("methods", JSONArray().apply {
      for (m in p.methods) put(JSONObject().apply { put("kind", m.kind); put("label", m.label); put("value", m.value) })
    })
    p.addresses?.let { list ->
      put("addresses", JSONArray().apply {
        for (a in list) put(JSONObject().apply {
          put("label", a.label); put("street", a.street); put("subLocality", a.subLocality); put("city", a.city)
          put("subAdministrativeArea", a.subAdministrativeArea); put("state", a.state); put("postalCode", a.postalCode)
          put("country", a.country); put("countryCode", a.countryCode); put("formatted", a.formatted)
        })
      })
    }
  }.toString()

  /** A base's addresses; none in a base kept before addresses were (they are then left alone). */
  private fun addressesFromJson(o: JSONObject): List<Address>? {
    val list = o.optJSONArray("addresses") ?: return null
    return (0 until list.length()).map { i ->
      val a = list.getJSONObject(i)
      fun str(k: String) = if (a.isNull(k)) null else a.optString(k)
      Address(
        label = str("label"), street = str("street"), subLocality = str("subLocality"), city = str("city"),
        subAdministrativeArea = str("subAdministrativeArea"), state = str("state"), postalCode = str("postalCode"),
        country = str("country"), countryCode = str("countryCode"), formatted = str("formatted") ?: "",
      )
    }
  }

  fun personFromJson(s: String): Person? = runCatching {
    val o = JSONObject(s)
    fun str(k: String) = if (o.isNull(k)) null else o.optString(k)
    val ms = o.optJSONArray("methods") ?: JSONArray()
    Person(
      guid = o.getString("guid"), etag = str("etag"), firstName = str("first"), middleName = str("middle"),
      lastName = str("last"), company = str("company"), isCompany = o.optBoolean("isCompany"),
      birthday = str("birthday"), notes = str("notes"),
      // Only the key is kept: which picture the phone has (the url is asked for anew each time).
      photo = str("photoKey")?.let { PhotoRef(url = "", crop = null, key = it) },
      prefix = str("prefix"), suffix = str("suffix"), nickname = str("nickname"),
      phoneticFirstName = str("phoneticFirst"), phoneticLastName = str("phoneticLast"),
      jobTitle = str("jobTitle"), department = str("department"),
      groups = o.optJSONArray("groups")?.let { a -> (0 until a.length()).map { a.getString(it) } } ?: emptyList(),
      methods = (0 until ms.length()).map { i ->
        val m = ms.getJSONObject(i)
        Method(m.getString("kind"), if (m.isNull("label")) null else m.optString("label"), m.getString("value"))
      },
      addresses = addressesFromJson(o),
    )
  }.getOrNull()
}

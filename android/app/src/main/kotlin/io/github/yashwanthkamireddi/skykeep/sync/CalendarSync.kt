package io.github.yashwanthkamireddi.skykeep.sync

import android.accounts.Account
import android.content.ContentProviderOperation
import android.content.ContentResolver
import android.content.ContentValues
import android.graphics.Color
import android.net.Uri
import android.provider.CalendarContract
import android.provider.CalendarContract.Calendars
import android.provider.CalendarContract.Events
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.ZoneOffset
import java.util.TimeZone
import org.json.JSONObject
import uniffi.quince_mobile.Cal
import uniffi.quince_mobile.CalendarWindow
import uniffi.quince_mobile.ChangeKind
import uniffi.quince_mobile.ChangeOutcome
import uniffi.quince_mobile.Ev
import uniffi.quince_mobile.EventChange
import uniffi.quince_mobile.RepeatRead
import uniffi.quince_mobile.readRepeat

/**
 * iCloud's calendars and Android's Calendar, both ways, for the span the engine reads: each
 * calendar a calendar under the Skykeep account (found again by Apple's id, `_SYNC_ID`), editable
 * here where iCloud lets it be; each event, and each occurrence of a repeating one, an event found
 * again the same way.
 *
 * Each sync sends first, then reads. Sending: what Android marks changed (DIRTY), new (no
 * `_SYNC_ID`) or deleted goes to iCloud through the engine — an edit as only what changed from the
 * event as last synced (its base, in SYNC_DATA2), one occurrence of a repeat changed alone, as
 * "This Event" on an iPhone. Reading: an event rewritten only when its etag (SYNC_DATA1) changed,
 * never one still waiting to be sent, and removed when gone from iCloud — events outside the span
 * are left alone, since nothing was read about them. Every event iCloud sends is matched against
 * all of the account's, wherever they start: matched only within the span, one that began before
 * it (or on its last day) was put in again on every sync.
 */
object CalendarSync {
  data class Result(val calendars: Int, val added: Int, val updated: Int, val removed: Int, val unchanged: Int, val held: Int)

  data class Sent(val sent: Int, val failed: Int, val later: Int, val reason: String?, val held: Int)

  /** The circuit breaker: more than this many events at once, and a tenth of them, waits. */
  private const val MASS = 25

  /** How rows are written here (SYNC_DATA3): an older one is rewritten from iCloud on the next sync. */

  /** An event made on this phone and not yet known to be in iCloud (SYNC_DATA5). */
  private const val MADE_HERE = "made-here"
  private const val FORMAT = "2"

  /** The span read: a month back and a year ahead, both days included, in the phone's zone. */
  fun span(today: LocalDate = LocalDate.now()): Pair<String, String> =
    today.minusMonths(1).toString() to today.plusMonths(12).toString()

  private fun Uri.asAdapter(a: Account) = buildUpon()
    .appendQueryParameter(CalendarContract.CALLER_IS_SYNCADAPTER, "true")
    .appendQueryParameter(Calendars.ACCOUNT_NAME, a.name)
    .appendQueryParameter(Calendars.ACCOUNT_TYPE, a.type)
    .build()

  fun sync(cr: ContentResolver, account: Account, window: CalendarWindow, from: String, to: String, confirmed: Boolean): Result =
    synchronized(this) { download(cr, account, window, from, to, confirmed) }

  fun send(cr: ContentResolver, account: Account, confirmed: Boolean, save: (List<EventChange>) -> List<ChangeOutcome>): Sent =
    synchronized(this) { upload(cr, account, confirmed, save) }

  // --- Sending ---------------------------------------------------------------------------------

  private data class Change(val rowId: Long, val change: EventChange)

  private fun upload(cr: ContentResolver, account: Account, confirmed: Boolean, save: (List<EventChange>) -> List<ChangeOutcome>): Sent {
    // Android's calendar row id → Apple's calendar id.
    val guidOf = HashMap<Long, String>()
    cr.query(
      Calendars.CONTENT_URI.asAdapter(account), arrayOf(Calendars._ID, Calendars._SYNC_ID),
      "${Calendars.ACCOUNT_TYPE}=? AND ${Calendars.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { c -> while (c.moveToNext()) c.getString(1)?.let { guidOf[c.getLong(0)] = it } }

    val changes = ArrayList<Change>()
    val dropped = ArrayList<Long>()
    val unknown = ArrayList<Long>()
    val refused = ArrayList<Pair<Long, String>>()
    cr.query(
      Events.CONTENT_URI.asAdapter(account),
      arrayOf(
        Events._ID, Events._SYNC_ID, Events.SYNC_DATA2, Events.DELETED, Events.CALENDAR_ID, Events.TITLE,
        Events.EVENT_LOCATION, Events.DESCRIPTION, Events.DTSTART, Events.DTEND, Events.DURATION,
        Events.ALL_DAY, Events.EVENT_TIMEZONE, Events.RRULE, Events.SYNC_DATA5,
      ),
      "${Events.ACCOUNT_TYPE}=? AND ${Events.ACCOUNT_NAME}=? AND (${Events.DIRTY}=1 OR ${Events.DELETED}=1)",
      arrayOf(account.type, account.name), null,
    )?.use { c ->
      while (c.moveToNext()) {
        val rowId = c.getLong(0)
        val sync = c.getString(1)
        val base = c.getString(2)?.let(::evFromJson)
        val deleted = c.getInt(3) != 0
        val made = c.getString(14) == MADE_HERE
        val calendar = guidOf[c.getLong(4)]
        fun ev(guid: String) = Ev(
          guid = guid, calendarGuid = calendar ?: "", title = c.getString(5) ?: "",
          location = c.getString(6)?.takeIf { it.isNotBlank() }, startsAt = 0, endsAt = 0,
          allDay = c.getInt(11) != 0, timezone = c.getString(12), notes = c.getString(7)?.takeIf { it.isNotBlank() },
          etag = base?.etag,
        ).let { e -> times(e, c.getLong(8), if (c.isNull(9)) null else c.getLong(9), c.getString(10)) }
        /** A new event sent as `guid`: its repeat read the way the engine reads an .ics rule. */
        fun create(guid: String) {
          val read = c.getString(13)?.takeIf { it.isNotBlank() }?.let { rule ->
            val e = ev("")
            readRepeat(rule, e.startsAt, e.allDay, e.timezone, ZoneId.systemDefault().id)
          }
          if (read is RepeatRead.Refused) {
            refused += rowId to "${read.reason} Change its repeat to one iCloud can keep, and it will be sent."
          } else {
            changes += Change(rowId, EventChange(ChangeKind.CREATE, guid, ev(guid), null, (read as? RepeatRead.Repeats)?.repeat))
          }
        }
        when {
          deleted && sync == null -> dropped += rowId
          // Made here and deleted before iCloud said it had it: deleted there too, if it got there.
          deleted && made -> changes += Change(rowId, EventChange(ChangeKind.DELETE, sync!!, null, null, null))
          // Made here, its first send not answered yet: sent again as the same new event. Without
          // its mark it read as an event this phone knew nothing of, was rewritten from iCloud,
          // which didn't have it, and was deleted.
          made && sync != null && calendar != null -> create(sync)
          // Changed with no base (an event written before bases were kept): never sent; read again.
          base == null && sync != null -> unknown += rowId
          deleted -> changes += Change(rowId, EventChange(ChangeKind.DELETE, sync!!, null, base, null))
          calendar == null -> refused += rowId to "This event's calendar isn't an iCloud calendar."
          sync == null -> {
            // New here: its id chosen now and kept, and marked as made here until iCloud has it,
            // so a send repeated after a lost answer finds the event made rather than making a
            // second, and a send that didn't land is made again, never taken for unknown.
            val guid = java.util.UUID.randomUUID().toString().uppercase()
            cr.update(
              Events.CONTENT_URI.asAdapter(account), ContentValues().apply { put(Events._SYNC_ID, guid); put(Events.SYNC_DATA5, MADE_HERE) },
              "${Events._ID}=?", arrayOf(rowId.toString()),
            )
            create(guid)
          }
          else -> changes += Change(rowId, EventChange(ChangeKind.UPDATE, sync, ev(sync), base, null))
        }
      }
    }
    for (id in dropped) removeRow(cr, account, id)
    for (id in unknown) restore(cr, account, id)
    for ((id, why) in refused) settled(cr, account, id, null, kept = why)
    val refusedReason = refused.firstOrNull()?.second
    if (changes.isEmpty()) return Sent(0, refused.size, 0, refusedReason, 0)
    if (!confirmed && changes.size > MASS && changes.size * 10 > count(cr, account)) {
      return Sent(0, refused.size, 0, refusedReason, changes.size)
    }
    val outcomes = save(changes.map { it.change })
    var sent = 0; var failed = refused.size; var later = 0; var reason = refusedReason
    for ((i, c) in changes.withIndex()) {
      when (val o = outcomes.getOrNull(i)) {
        is ChangeOutcome.Saved -> {
          sent++
          if (c.change.kind == ChangeKind.DELETE) removeRow(cr, account, c.rowId) else settled(cr, account, c.rowId, null)
        }
        is ChangeOutcome.Recreated -> { sent++; settled(cr, account, c.rowId, o.guid) }
        is ChangeOutcome.Failed -> {
          reason = o.reason
          if (o.later) later++ else { failed++; settled(cr, account, c.rowId, null, kept = o.reason) }
        }
        null -> later++
      }
    }
    return Sent(sent, failed, later, reason, 0)
  }

  /**
   * An event's moments as the engine keeps them: a timed one as the instant; an all-day one as
   * midnight on this phone's clock of its date, where Android keeps the date at UTC midnight —
   * the inverse of [values]. An end given as a DURATION (a repeating event's) is worked out.
   */
  private fun times(e: Ev, startMs: Long, endMs: Long?, duration: String?): Ev {
    val end = endMs ?: (startMs + (duration?.let(::durationMs) ?: 3_600_000))
    return if (e.allDay) {
      fun local(ms: Long) = Instant.ofEpochMilli(ms).atZone(ZoneOffset.UTC).toLocalDate()
        .atStartOfDay(ZoneId.systemDefault()).toEpochSecond()
      e.copy(startsAt = local(startMs), endsAt = local(end), timezone = null)
    } else {
      e.copy(startsAt = startMs / 1000, endsAt = end / 1000)
    }
  }

  /**
   * An RFC 5545 duration in milliseconds: "P1W", "P1D", "PT1H30M", "P1DT2H", and the "P3600S"
   * Android's own Calendar writes (seconds with no T). None for anything else.
   */
  private fun durationMs(d: String): Long? {
    Regex("^P(\\d+)S$").matchEntire(d)?.let { return it.groupValues[1].toLong() * 1000 }
    val m = Regex("^P(?:(\\d+)W)?(?:(\\d+)D)?(?:T(?:(\\d+)H)?(?:(\\d+)M)?(?:(\\d+)S)?)?$").matchEntire(d) ?: return null
    val (w, day, h, min, sec) = m.destructured
    fun n(x: String) = x.toLongOrNull() ?: 0
    val total = ((n(w) * 7 + n(day)) * 86_400 + n(h) * 3_600 + n(min) * 60 + n(sec)) * 1000
    return total.takeIf { it > 0 }
  }

  /**
   * Sent, or refused: no longer waiting. A refusal's reason is kept on the row (SYNC_DATA4) for as
   * long as iCloud doesn't have it, so the app says so every time rather than once; a row written
   * from iCloud, or sent, has none.
   */
  private fun settled(cr: ContentResolver, account: Account, rowId: Long, newGuid: String?, kept: String? = null) {
    cr.update(
      Events.CONTENT_URI.asAdapter(account),
      ContentValues().apply {
        put(Events.DIRTY, 0)
        putNull(Events.SYNC_DATA1)
        if (kept != null) put(Events.SYNC_DATA4, kept) else putNull(Events.SYNC_DATA4)
        newGuid?.let { put(Events._SYNC_ID, it) }
      },
      "${Events._ID}=?", arrayOf(rowId.toString()),
    )
  }

  /** Events iCloud couldn't take that stay on this phone alone, and the first one's reason. */
  fun onlyHere(cr: ContentResolver, account: Account): Pair<Int, String?> =
    cr.query(
      Events.CONTENT_URI.asAdapter(account), arrayOf(Events.SYNC_DATA4),
      "${Events.ACCOUNT_TYPE}=? AND ${Events.ACCOUNT_NAME}=? AND ${Events.DELETED}=0 AND ${Events.SYNC_DATA4} IS NOT NULL",
      arrayOf(account.type, account.name), null,
    )?.use { c -> c.count to (if (c.moveToFirst()) c.getString(0) else null) } ?: (0 to null)

  private fun restore(cr: ContentResolver, account: Account, rowId: Long) {
    cr.update(
      Events.CONTENT_URI.asAdapter(account),
      ContentValues().apply { put(Events.DIRTY, 0); put(Events.DELETED, 0); putNull(Events.SYNC_DATA1) },
      "${Events._ID}=?", arrayOf(rowId.toString()),
    )
  }

  private fun removeRow(cr: ContentResolver, account: Account, rowId: Long) {
    cr.delete(Events.CONTENT_URI.asAdapter(account), "${Events._ID}=?", arrayOf(rowId.toString()))
  }

  private fun count(cr: ContentResolver, account: Account): Int =
    cr.query(
      Events.CONTENT_URI.asAdapter(account), arrayOf(Events._ID),
      "${Events.ACCOUNT_TYPE}=? AND ${Events.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { it.count } ?: 0

  // --- Reading ---------------------------------------------------------------------------------

  private fun download(cr: ContentResolver, account: Account, window: CalendarWindow, from: String, to: String, confirmed: Boolean): Result {
    val ids = calendars(cr, account, window.calendars)
    val zone = ZoneId.systemDefault()
    // The span's bounds as moments: its first day's start to the day after its last. An
    // all-day event is kept at UTC midnight, so it is held to the span by its date in UTC.
    val first = LocalDate.parse(from)
    val after = LocalDate.parse(to).plusDays(1)
    val start = first.atStartOfDay(zone).toInstant().toEpochMilli()
    val end = after.atStartOfDay(zone).toInstant().toEpochMilli()
    val startUtc = first.atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
    val endUtc = after.atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()

    data class Local(val id: Long, val etag: String?, val starts: Long, val allDay: Boolean, val pending: Boolean, val format: String?)
    val local = HashMap<String, Local>()
    val extra = ArrayList<Long>()
    cr.query(
      Events.CONTENT_URI.asAdapter(account),
      arrayOf(Events._ID, Events._SYNC_ID, Events.SYNC_DATA1, Events.DTSTART, Events.ALL_DAY, Events.DIRTY, Events.DELETED, Events.SYNC_DATA3),
      "${Events.ACCOUNT_TYPE}=? AND ${Events.ACCOUNT_NAME}=?",
      arrayOf(account.type, account.name), null,
    )?.use { c ->
      while (c.moveToNext()) {
        val sync = c.getString(1) ?: continue
        val pending = c.getInt(5) != 0 || c.getInt(6) != 0
        if (sync in local) { if (!pending) extra += c.getLong(0); continue }
        local[sync] = Local(c.getLong(0), c.getString(2), c.getLong(3), c.getInt(4) != 0, pending, c.getString(7))
      }
    }
    fun inSpan(l: Local) = if (l.allDay) l.starts in startUtc until endUtc else l.starts in start until end

    var added = 0; var updated = 0; var unchanged = 0
    val ops = ArrayList<ContentProviderOperation>()
    fun flush() { if (ops.isNotEmpty()) { cr.applyBatch(CalendarContract.AUTHORITY, ops); ops.clear() } }
    for (e in window.events) {
      val calendarId = ids[e.calendarGuid] ?: continue
      val have = local.remove(e.guid)
      // Changed here and not yet in iCloud: left as it is, sent on the next sync.
      if (have != null && have.pending) continue
      if (have != null && have.etag != null && have.etag == e.etag && have.format == FORMAT) { unchanged++; continue }
      if (ops.size > 300) flush()
      val values = values(e, calendarId)
      ops += if (have == null) {
        added++
        ContentProviderOperation.newInsert(Events.CONTENT_URI.asAdapter(account)).withValues(values).build()
      } else {
        updated++
        ContentProviderOperation.newUpdate(Events.CONTENT_URI.asAdapter(account))
          .withSelection("${Events._ID}=?", arrayOf(have.id.toString())).withValues(values).build()
      }
    }
    // Gone from iCloud: those in the span that iCloud did not send (never one waiting to be
    // sent). Outside it, left alone. Far fewer than here at once is held until confirmed.
    val gone = local.values.filter { !it.pending && inSpan(it) }.map { it.id }
    val massive = gone.size > MASS && gone.size * 10 > (window.events.size + gone.size)
    val removing = if (massive && !confirmed) emptyList() else gone
    for (id in removing + extra) {
      if (ops.size > 300) flush()
      ops += ContentProviderOperation.newDelete(Events.CONTENT_URI.asAdapter(account))
        .withSelection("${Events._ID}=?", arrayOf(id.toString())).build()
    }
    flush()
    return Result(ids.size, added, updated, removing.size + extra.size, unchanged, gone.size - removing.size)
  }

  /** The account's calendars made to match iCloud's; Apple's id → Android's row id. */
  private fun calendars(cr: ContentResolver, account: Account, wanted: List<Cal>): Map<String, Long> {
    val have = HashMap<String, Long>()
    cr.query(
      Calendars.CONTENT_URI.asAdapter(account), arrayOf(Calendars._ID, Calendars._SYNC_ID),
      "${Calendars.ACCOUNT_TYPE}=? AND ${Calendars.ACCOUNT_NAME}=?", arrayOf(account.type, account.name), null,
    )?.use { c -> while (c.moveToNext()) c.getString(1)?.let { have[it] = c.getLong(0) } }
    val out = HashMap<String, Long>()
    for (k in wanted) {
      val values = ContentValues().apply {
        put(Calendars.CALENDAR_DISPLAY_NAME, k.title)
        put(Calendars.CALENDAR_COLOR, k.colour?.let { runCatching { Color.parseColor(it) }.getOrNull() } ?: Color.rgb(0x1E, 0x88, 0xE5))
        // Editable here where iCloud lets it be; a subscription or a calendar shared read-only
        // stays read-only.
        put(Calendars.CALENDAR_ACCESS_LEVEL, if (k.writable) Calendars.CAL_ACCESS_OWNER else Calendars.CAL_ACCESS_READ)
        put(Calendars.SYNC_EVENTS, 1)
      }
      val id = have.remove(k.guid)
      if (id != null) {
        cr.update(Calendars.CONTENT_URI.asAdapter(account), values, "${Calendars._ID}=?", arrayOf(id.toString()))
        out[k.guid] = id
      } else {
        values.put(Calendars.ACCOUNT_NAME, account.name)
        values.put(Calendars.ACCOUNT_TYPE, account.type)
        values.put(Calendars.NAME, k.guid)
        values.put(Calendars._SYNC_ID, k.guid)
        values.put(Calendars.OWNER_ACCOUNT, account.name)
        // Shown when it first arrives; after that, shown or hidden as the person chose here.
        values.put(Calendars.VISIBLE, 1)
        values.put(Calendars.CALENDAR_TIME_ZONE, TimeZone.getDefault().id)
        cr.insert(Calendars.CONTENT_URI.asAdapter(account), values)?.lastPathSegment?.toLongOrNull()?.let { out[k.guid] = it }
      }
    }
    // A calendar gone from iCloud goes, its events with it.
    for (id in have.values) {
      cr.delete(Calendars.CONTENT_URI.asAdapter(account), "${Calendars._ID}=?", arrayOf(id.toString()))
    }
    return out
  }

  private fun values(e: Ev, calendarId: Long) = ContentValues().apply {
    put(Events.CALENDAR_ID, calendarId)
    put(Events._SYNC_ID, e.guid)
    put(Events.SYNC_DATA1, e.etag)
    put(Events.SYNC_DATA2, evJson(e))
    put(Events.SYNC_DATA3, FORMAT)
    putNull(Events.SYNC_DATA4)
    // In iCloud now (it came back from it): no longer one made here and waiting.
    putNull(Events.SYNC_DATA5)
    put(Events.TITLE, e.title)
    put(Events.EVENT_LOCATION, e.location)
    put(Events.DESCRIPTION, e.notes)
    if (e.allDay) {
      // The engine gives an all-day event as midnight on this phone's clock; Android keeps
      // them as the same dates at midnight UTC, or they show a day early west of Greenwich.
      fun utcMidnight(seconds: Long): Long {
        val day = Instant.ofEpochSecond(seconds).atZone(ZoneId.systemDefault()).toLocalDate()
        return day.atStartOfDay(ZoneOffset.UTC).toInstant().toEpochMilli()
      }
      val startDay = utcMidnight(e.startsAt)
      put(Events.DTSTART, startDay)
      put(Events.DTEND, maxOf(utcMidnight(e.endsAt), startDay + 86_400_000))
      put(Events.ALL_DAY, 1)
      put(Events.EVENT_TIMEZONE, "UTC")
    } else {
      put(Events.DTSTART, e.startsAt * 1000)
      put(Events.DTEND, e.endsAt * 1000)
      put(Events.ALL_DAY, 0)
      put(Events.EVENT_TIMEZONE, e.timezone ?: TimeZone.getDefault().id)
    }
  }

  // --- The base, as kept in SYNC_DATA2 -------------------------------------------------------

  private fun evJson(e: Ev): String = JSONObject().apply {
    put("guid", e.guid); put("calendar", e.calendarGuid); put("title", e.title); put("location", e.location)
    put("starts", e.startsAt); put("ends", e.endsAt); put("allDay", e.allDay); put("tz", e.timezone)
    put("notes", e.notes); put("etag", e.etag)
  }.toString()

  private fun evFromJson(s: String): Ev? = runCatching {
    val o = JSONObject(s)
    fun str(k: String) = if (o.isNull(k)) null else o.optString(k)
    Ev(
      guid = o.getString("guid"), calendarGuid = o.getString("calendar"), title = o.optString("title"),
      location = str("location"), startsAt = o.getLong("starts"), endsAt = o.getLong("ends"),
      allDay = o.optBoolean("allDay"), timezone = str("tz"), notes = str("notes"), etag = str("etag"),
    )
  }.getOrNull()
}

package io.github.yashwanthkamireddi.skykeep.sync

import android.content.Context
import android.content.SharedPreferences

/**
 * What the last sync of Contacts and of Calendar did, kept for the app's screen: Android runs
 * the syncs (the adapters in Services.kt) and says only whether one is running, never how it
 * went. The adapters write it here; the screen reads it and is told when it changes.
 */
object Outcome {
  private const val FILE = "sync-outcome"

  /**
   * `okAt`: the last time it synced cleanly, which the error wording names. `onlyHere`: how many
   * items iCloud couldn't take stay on this phone alone, and `why` (the first one's reason): said
   * for as long as it is true, not only by the sync that found it.
   */
  data class Last(
    val at: Long, val ok: Boolean, val said: String, val okAt: Long, val onlyHere: Int = 0, val why: String? = null,
    /** Advanced Data Protection holds it until the person approves: waiting, not failed. */
    val paused: Boolean = false,
  )

  private fun prefs(context: Context): SharedPreferences =
    context.applicationContext.getSharedPreferences(FILE, Context.MODE_PRIVATE)

  fun record(
    context: Context, authority: String, ok: Boolean, said: String, held: Int = 0,
    onlyHere: Int = 0, why: String? = null, paused: Boolean = false,
  ) {
    prefs(context).edit()
      .putLong("$authority.at", System.currentTimeMillis())
      .putBoolean("$authority.ok", ok)
      .putString("$authority.said", said)
      .putInt("$authority.held", held)
      .putInt("$authority.onlyHere", onlyHere)
      .putString("$authority.why", why)
      .putBoolean("$authority.paused", paused)
      // Waiting on an approval read nothing: the last clean sync stays the one before it.
      .apply { if (ok && !paused) putLong("$authority.okAt", System.currentTimeMillis()) }
      .apply()
  }

  /** How many cards the last sync held back (iCloud listed far fewer than the phone has). */
  fun held(context: Context, authority: String): Int = prefs(context).getInt("$authority.held", 0)

  /** The person confirmed those removals: the next sync makes them, once. */
  fun confirmMassRemoval(context: Context, authority: String) {
    prefs(context).edit().putBoolean("$authority.confirmed", true).apply()
  }

  /** Whether removals were confirmed — and, asked, the confirmation is spent. */
  fun massRemovalConfirmed(context: Context, authority: String): Boolean {
    val p = prefs(context)
    val yes = p.getBoolean("$authority.confirmed", false)
    if (yes) p.edit().putBoolean("$authority.confirmed", false).apply()
    return yes
  }

  fun last(context: Context, authority: String): Last? {
    val p = prefs(context)
    val said = p.getString("$authority.said", null) ?: return null
    return Last(
      p.getLong("$authority.at", 0), p.getBoolean("$authority.ok", false), said, p.getLong("$authority.okAt", 0),
      p.getInt("$authority.onlyHere", 0), p.getString("$authority.why", null),
      p.getBoolean("$authority.paused", false),
    )
  }

  /** Told of every new outcome until the returned function is called. */
  fun watch(context: Context, changed: () -> Unit): () -> Unit {
    val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> changed() }
    val p = prefs(context)
    p.registerOnSharedPreferenceChangeListener(listener)
    return { p.unregisterOnSharedPreferenceChangeListener(listener) }
  }
}

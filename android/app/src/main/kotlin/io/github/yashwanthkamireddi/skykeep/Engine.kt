package io.github.yashwanthkamireddi.skykeep

import android.content.Context
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.start

/**
 * The engine, once per process: the app's screens and Android's sync adapters share one
 * signed-in Account, so a sync started by Android uses the same session the app shows.
 */
object Engine {
  @Volatile private var account: Account? = null

  fun account(context: Context): Account = account ?: synchronized(this) {
    account ?: run {
      val app = context.applicationContext
      start(KeystoreSecrets(app))
      Account(app.filesDir.resolve("account").path).also {
        it.driveParts(app)
        account = it
      }
    }
  }

  /**
   * Where Drive's parts read for other apps are kept: the app's cache, at most half the share
   * of storage Android suggests for this app's cache (StorageManager.getCacheQuotaBytes), between
   * 64 MB and 1 GB. Past it, the parts read longest ago are let go of; Android may also clear
   * them when storage runs low, and they are fetched again when read again.
   */
  private fun Account.driveParts(app: Context) {
    val dir = app.cacheDir.resolve("drive-parts")
    val quota = runCatching {
      val storage = app.getSystemService(android.os.storage.StorageManager::class.java)
      storage.getCacheQuotaBytes(storage.getUuidForPath(app.cacheDir))
    }.getOrDefault(1L shl 30)
    drivePartsAt(dir.path, (quota / 2).coerceIn(64L shl 20, 1L shl 30).toULong())
  }
}

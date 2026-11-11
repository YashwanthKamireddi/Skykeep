package io.github.yashwanthkamireddi.skykeep.ui

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import io.github.yashwanthkamireddi.skykeep.R
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** One licence and what in Skykeep is under it. */
private class Notice(val title: String, val users: String, val text: String)

/**
 * Every licence the app ships, read from what the build wrote into the app: the engine's crates
 * (cargo-about), the Android libraries (AboutLibraries) and the icons (Lucide). Nothing is typed
 * by hand, so the list is always the build's own.
 */
private fun notices(context: Context): List<Notice> {
  val res = context.resources
  fun raw(name: String) = res.getIdentifier(name, "raw", context.packageName).takeIf { it != 0 }
    ?.let { id -> res.openRawResource(id).bufferedReader().use { it.readText() } }
  val out = ArrayList<Notice>()
  val texts = HashMap<String, String>()
  raw("engine_licences")?.let { json ->
    val all = JSONObject(json).getJSONArray("licenses")
    for (i in 0 until all.length()) {
      val l = all.getJSONObject(i)
      val used = l.getJSONArray("used_by")
      val crates = (0 until used.length()).map { used.getJSONObject(it).getJSONObject("crate").let { c -> "${c.getString("name")} ${c.getString("version")}" } }
      texts.putIfAbsent(l.getString("id"), l.getString("text"))
      out += Notice(l.getString("name"), "Engine: " + crates.joinToString(", "), l.getString("text"))
    }
  }
  raw("aboutlibraries")?.let { json ->
    val libs = JSONObject(json).getJSONArray("libraries")
    val byLicence = LinkedHashMap<String, MutableList<String>>()
    for (i in 0 until libs.length()) {
      val lib = libs.getJSONObject(i)
      val ls = lib.optJSONArray("licenses")
      // JNA is offered as Apache-2.0 or LGPL; Skykeep takes it under Apache-2.0, as the rest.
      val key = (0 until (ls?.length() ?: 0)).map { ls!!.getString(it) }.let { if ("Apache-2.0" in it) "Apache-2.0" else it.firstOrNull() ?: "Unknown" }
      byLicence.getOrPut(key) { mutableListOf() } += "${lib.getString("name")} ${lib.optString("artifactVersion")}"
    }
    for ((licence, names) in byLicence) {
      out += Notice(licence, "Android: " + names.joinToString(", "), texts[licence] ?: "See https://spdx.org/licenses/$licence.html")
    }
  }
  raw("lucide_license")?.let { out += Notice("ISC License", "Icons: Lucide", it) }
  return out
}

/** Settings › Open-source licences: each licence, what uses it, and its full text on a tap. */
@Composable
fun LicencesPage(onBack: () -> Unit) {
  val context = LocalContext.current
  BackHandler(onBack = onBack)
  val all by produceState<List<Notice>?>(null) { value = withContext(Dispatchers.IO) { notices(context) } }
  var open by remember { mutableStateOf<Notice?>(null) }
  open?.let { n ->
    BackHandler { open = null }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(horizontal = 20.dp, vertical = 8.dp)) {
      item { IconButton(onClick = { open = null }) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Licences") } }
      item { Text(n.title, style = MaterialTheme.typography.headlineMedium, modifier = Modifier.padding(bottom = 8.dp)) }
      item { Text(n.users, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(bottom = 16.dp)) }
      // Long texts in pieces, so a 30 kB licence scrolls without one huge text block.
      items(n.text.split("\n\n")) { para ->
        Text(para, style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace), modifier = Modifier.padding(bottom = 10.dp))
      }
    }
    return
  }
  LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(horizontal = 16.dp, vertical = 8.dp)) {
    item { IconButton(onClick = onBack) { Icon(painterResource(R.drawable.ic_back), contentDescription = "Settings") } }
    item {
      Text("Open-source licences", style = MaterialTheme.typography.headlineLarge, modifier = Modifier.padding(start = 4.dp, bottom = 8.dp))
      Text(
        "Skykeep is free software under the GPL-3.0. It is built on the work below, each under its own licence.",
        style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(start = 4.dp, bottom = 16.dp),
      )
    }
    val list = all.orEmpty()
    item {
      GroupCard(list, inset = 16) { n ->
        Column(Modifier.fillMaxWidth().clickable { open = n }.padding(horizontal = 16.dp, vertical = 12.dp)) {
          Text(n.title, style = MaterialTheme.typography.bodyLarge)
          Text(n.users, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis)
        }
      }
    }
  }
}

package io.github.yashwanthkamireddi.skykeep

import android.content.Context
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import io.github.yashwanthkamireddi.skykeep.sync.SessionEnded
import io.github.yashwanthkamireddi.skykeep.ui.MainScreens
import io.github.yashwanthkamireddi.skykeep.ui.Setup
import io.github.yashwanthkamireddi.skykeep.ui.Welcome
import io.github.yashwanthkamireddi.skykeep.ui.allowed
import io.github.yashwanthkamireddi.skykeep.ui.phoneAccount
import io.github.yashwanthkamireddi.skykeep.ui.theme.SkykeepTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.Reopened

/** Where the app is: starting; first-run welcome; signing in; setting up the phone; in use. */
private enum class Stage { Starting, Welcome, SignIn, Setup, Main }

/** Whether this phone has seen the welcome: shown once, before the first sign-in. */
private fun welcomed(context: Context) =
  context.getSharedPreferences("settings", Context.MODE_PRIVATE).getBoolean("welcomed", false)

private fun markWelcomed(context: Context) =
  context.getSharedPreferences("settings", Context.MODE_PRIVATE).edit().putBoolean("welcomed", true).apply()

class MainActivity : ComponentActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    enableEdgeToEdge()
    setContent {
      SkykeepTheme {
        Surface(modifier = Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
          var stage by remember { mutableStateOf(Stage.Starting) }
          var engine by remember { mutableStateOf<Account?>(null) }
          LaunchedEffect(Unit) {
            // From the phone alone: a kept session opens the library at once, offline included;
            // Apple is asked behind it, and only Apple refusing the session (or Apple wanting a
            // code) brings sign-in up. The screen never waits on the network to draw.
            val (a, kept) = withContext(Dispatchers.IO) {
              val a = Engine.account(applicationContext)
              a to a.hasSession()
            }
            engine = a
            // Opened from "Sign in to Skykeep again": Apple ended the session, so sign-in is
            // where the person is going, whatever is kept.
            // (An old notification tapped after signing in again opens the library as usual.)
            val signInAsked = intent?.getBooleanExtra(SessionEnded.SIGN_IN, false) == true &&
              withContext(Dispatchers.IO) { a.accountTag() }?.let { SessionEnded.ended(applicationContext, it) } == true
            stage = when {
              !kept -> if (welcomed(applicationContext)) Stage.SignIn else Stage.Welcome
              signInAsked -> Stage.SignIn
              // Signed in, but the phone not set up yet (no account here, or no access).
              phoneAccount(applicationContext) == null || !allowed(applicationContext) -> Stage.Setup
              else -> Stage.Main
            }
            if (kept) {
              val where = withContext(Dispatchers.IO) { runCatching { a.reopen() }.getOrNull() }
              // A failure to ask (not Apple's answer) leaves the library open: the next sync tries again.
              if (where == Reopened.SIGN_IN || where == Reopened.CODE) stage = Stage.SignIn
            }
            // An account made before something synced (Reminders, 10 Nov) is given it now.
            // Only the signed-in Apple Account's own phone account, never another's kept here.
            phoneAccount(applicationContext)?.let { existing ->
              withContext(Dispatchers.IO) {
                val tag = a.accountTag()
                if (tag != null && io.github.yashwanthkamireddi.skykeep.sync.syncsFor(applicationContext, a, existing)) {
                  io.github.yashwanthkamireddi.skykeep.sync.ensureAccount(applicationContext, existing.name, tag)
                }
              }
            }
            // Drive saves an earlier run held and had not sent yet go now.
            if (kept) io.github.yashwanthkamireddi.skykeep.sync.DriveWrites.resume(applicationContext)
          }
          when (stage) {
            Stage.Starting -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
            Stage.Welcome -> Welcome { markWelcomed(applicationContext); stage = Stage.SignIn }
            Stage.SignIn -> SignInScreen(engine!!) {
              // Signed in again: background syncing goes on (Setup gives the account its work
              // back) and "Sign in again" is taken down.
              SessionEnded.clear(applicationContext)
              stage = Stage.Setup
            }
            Stage.Setup -> Setup { stage = Stage.Main }
            Stage.Main -> MainScreens(engine!!) { stage = Stage.SignIn }
          }
        }
      }
    }
  }
}

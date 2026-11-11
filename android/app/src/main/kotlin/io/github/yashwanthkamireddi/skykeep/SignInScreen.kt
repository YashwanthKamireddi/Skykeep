package io.github.yashwanthkamireddi.skykeep

import io.github.yashwanthkamireddi.skykeep.R
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.platform.LocalContext
import androidx.compose.material3.Surface
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.Image
import android.net.Uri
import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.runtime.LaunchedEffect
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.quince_mobile.Account
import uniffi.quince_mobile.EngineException
import uniffi.quince_mobile.SignIn

private sealed interface Step {
  data object Credentials : Step
  data object Code : Step
}

/**
 * Signing in with the Apple Account: the desktop app's own sign-in, in Android's form. Opaque and
 * plain (never over glass), Skykeep's own mark at the top, and exactly what happens to the
 * password said under the fields. Fields carry autofill hints, so a password manager fills
 * them; the code field takes the six digits from the SMS keyboard suggestion. Every call runs
 * off the main thread.
 */
@Composable
fun SignInScreen(account: Account, onSignedIn: () -> Unit) {
  val context = LocalContext.current
  var step by remember { mutableStateOf<Step>(Step.Credentials) }
  var appleId by remember { mutableStateOf("") }
  var password by remember { mutableStateOf("") }
  var code by remember { mutableStateOf("") }
  var busy by remember { mutableStateOf(false) }
  var error by remember { mutableStateOf<String?>(null) }
  val scope = rememberCoroutineScope()

  fun run(block: () -> Unit) {
    if (busy) return
    busy = true
    error = null
    scope.launch {
      val failure = withContext(Dispatchers.IO) {
        try { block(); null } catch (e: EngineException.Failed) { e.reason }
      }
      busy = false
      error = failure
    }
  }

  Column(
    Modifier.fillMaxSize().systemBarsPadding().imePadding().verticalScroll(rememberScrollState())
      .padding(horizontal = 28.dp).padding(top = 32.dp, bottom = 24.dp).widthIn(max = 460.dp),
    verticalArrangement = Arrangement.spacedBy(16.dp),
  ) {
    Image(painterResource(R.mipmap.ic_launcher_foreground), contentDescription = "Skykeep", modifier = Modifier.size(88.dp).offset(x = (-16).dp))
    when (step) {
      Step.Credentials -> {
        Text("Sign in with your Apple Account", style = MaterialTheme.typography.headlineMedium)
        OutlinedTextField(
          value = appleId, onValueChange = { appleId = it }, label = { Text("Email or phone number") },
          singleLine = true, enabled = !busy,
          keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Email, imeAction = ImeAction.Next),
          modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Username },
        )
        val submit = {
          run {
            when (account.signIn(appleId.trim(), password)) {
              SignIn.COMPLETE -> scope.launch { onSignedIn() }
              SignIn.NEEDS_CODE -> step = Step.Code
            }
            password = ""
          }
        }
        var shown by remember { mutableStateOf(false) }
        OutlinedTextField(
          value = password, onValueChange = { password = it }, label = { Text("Password") },
          singleLine = true, enabled = !busy,
          visualTransformation = if (shown) VisualTransformation.None else PasswordVisualTransformation(),
          trailingIcon = {
            TextButton(onClick = { shown = !shown }) { Text(if (shown) "Hide" else "Show") }
          },
          keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Go),
          keyboardActions = KeyboardActions(onGo = { submit() }),
          modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.Password },
        )
        Button(
          onClick = { submit() }, enabled = !busy && appleId.isNotBlank() && password.isNotEmpty(),
          modifier = Modifier.fillMaxWidth().height(52.dp),
        ) { if (busy) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = MaterialTheme.colorScheme.onPrimary) else Text("Sign In") }
        error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium) }
        TextButton(onClick = {
          runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse("https://iforgot.apple.com")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
        }) { Text("Forgot password? (opens Apple's page)") }
        // What happens to the password, each part literally true of this build.
        Surface(color = MaterialTheme.colorScheme.surfaceContainer, shape = MaterialTheme.shapes.large) {
          Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("How your password is handled", style = MaterialTheme.typography.titleSmall)
            Text(
              "Skykeep signs in the way Apple's own sign-in does (SRP): it proves your password to Apple without sending it. To stay signed in, it keeps your password and session sealed with a key in this phone's Android Keystore, left out of backups, and erases them when you sign out. It talks only to Apple.",
              style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
          }
        }
        Text("Skykeep is not made by or affiliated with Apple.", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
      }
      Step.Code -> {
        // Where Apple can send a code: its devices, or a trusted phone number by text. Someone
        // who left the iPhone for Android may have no Apple device at all, and could not sign in
        // with a code that only went to one.
        var options by remember { mutableStateOf<uniffi.quince_mobile.CodeOptions?>(null) }
        var texted by remember { mutableStateOf<uniffi.quince_mobile.TrustedPhone?>(null) }
        LaunchedEffect(Unit) {
          options = withContext(Dispatchers.IO) { runCatching { account.codeOptions() }.getOrNull() }
        }
        val o = options
        val devices = o == null || o.trustedDevices > 0u
        Text("Enter the code", style = MaterialTheme.typography.headlineMedium)
        Text(
          when {
            o?.securityKey == true -> "This Apple Account signs in with a security key, which Skykeep can't use yet. Sign in at icloud.com in a browser with your key instead."
            o?.tooManyCodesSent == true -> "Apple has sent too many codes for now. Wait a while, then try again."
            texted != null -> "Apple sent a six-digit code by text to ${texted!!.maskedNumber}. Type it here."
            devices -> "Apple sent a six-digit code to your iPhone, iPad or Mac. Allow the sign-in there, then type the code here."
            else -> "Apple can't send a code to an Apple device for this account. Get one by text message instead."
          },
          style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        val submit = {
          run {
            val phone = texted
            if (phone != null) account.submitSmsCode(phone.id, code) else account.submitCode(code)
            scope.launch { onSignedIn() }
          }
        }
        OutlinedTextField(
          value = code, onValueChange = { v -> code = v.filter(Char::isDigit).take(6); if (code.length == 6) submit() },
          label = { Text("Code") }, singleLine = true, enabled = !busy,
          textStyle = MaterialTheme.typography.headlineSmall.copy(letterSpacing = androidx.compose.ui.unit.TextUnit(0.3f, androidx.compose.ui.unit.TextUnitType.Em)),
          keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.NumberPassword, imeAction = ImeAction.Done),
          modifier = Modifier.fillMaxWidth().semantics { contentType = ContentType.SmsOtpCode },
        )
        if (busy) CircularProgressIndicator(Modifier.size(24.dp))
        error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium) }
        TextButton(
          onClick = { run { texted?.let { account.sendSms(it.id) } ?: account.resendToDevices() } },
          enabled = !busy && o?.securityKey != true,
        ) { Text("Send a new code") }
        // Each trusted number Apple will text, the way icloud.com offers them.
        if (o != null && !o.securityKey) for (phone in o.phones) {
          if (texted?.id == phone.id) continue
          TextButton(onClick = { run { account.sendSms(phone.id); texted = phone; code = "" } }, enabled = !busy) {
            Text("Text a code to ${phone.maskedNumber}")
          }
        }
        TextButton(onClick = { step = Step.Credentials; code = ""; error = null }, enabled = !busy) { Text("Use a different account") }
      }
    }
  }
}

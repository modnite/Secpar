package com.pcremote.account

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.autofill.ContentType
import androidx.compose.ui.semantics.contentType
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import com.pcremote.ui.components.*
import com.pcremote.ui.theme.LocalMidnight

enum class SignInStep { SIGN_IN, SIGN_UP, CODE, FORGOT }

/** One request the form wants sent, and what to check before sending it. Pure, so tests can run it. */
object SignInRules {
    fun emailOk(email: String): Boolean {
        val e = email.trim()
        val at = e.indexOf('@')
        return e.length <= 254 && at > 0 && e.indexOf('.', at) > at + 1 && !e.endsWith(".") && e.none { it.isWhitespace() }
    }
    fun passwordOk(p: String) = p.codePointCount(0, p.length) in 8..256 && p.length <= 256
    fun handleOk(h: String) = h.length in 3..30 && h.all { it.isLetterOrDigit() && it.code < 128 || it == '_' }
    fun codeOf(raw: String) = raw.filterNot { it.isWhitespace() }

    /** Plain words for a problem the app can see before asking the service. Null means fine. */
    fun problem(step: SignInStep, reset: Boolean, email: String, password: String, handle: String, code: String): String? = when {
        step == SignInStep.SIGN_IN && !emailOk(email) -> "Enter your email address."
        step == SignInStep.SIGN_IN && password.isEmpty() -> "Enter your password."
        step == SignInStep.SIGN_UP && !emailOk(email) -> "Enter a valid email address."
        step == SignInStep.SIGN_UP && !handleOk(handle) -> "Use a username of 3 to 30 letters, numbers or underscores."
        step == SignInStep.SIGN_UP && !passwordOk(password) -> "Use a password of 8 to 256 characters."
        step == SignInStep.FORGOT && !emailOk(email) -> "Enter a valid email address."
        step == SignInStep.CODE && !(codeOf(code).length == 6 && codeOf(code).all { it in '0'..'9' }) -> "Enter the six digit code from the email."
        step == SignInStep.CODE && reset && !passwordOk(password) -> "Use a password of 8 to 256 characters."
        else -> null
    }
}

/** Sign in, create an account and recover a password, all inside the app. Shared by Computers and Friends. */
@Composable
fun SignInForm(acts: AccountActions, busy: Boolean, modifier: Modifier = Modifier) {
    val c = LocalMidnight.current
    var step by rememberSaveable { mutableStateOf(SignInStep.SIGN_IN) }
    var reset by rememberSaveable { mutableStateOf(false) }
    var email by rememberSaveable { mutableStateOf("") }
    var password by rememberSaveable { mutableStateOf("") }
    var handle by rememberSaveable { mutableStateOf("") }
    var code by rememberSaveable { mutableStateOf("") }
    var error by rememberSaveable { mutableStateOf("") }
    var info by rememberSaveable { mutableStateOf("") }
    fun go(next: SignInStep, forReset: Boolean = false) { step = next; reset = forReset; error = ""; info = "" }
    fun signIn() = acts.signIn(email.trim(), password) { note -> if (note != null) error = note }
    fun send(route: String, body: Map<String, String>, then: (() -> Unit)? = null) =
        acts.auth(route, body) { note ->
            if (note != null) error = note else then?.invoke()
        }
    fun sent(forReset: Boolean) { step = SignInStep.CODE; reset = forReset; code = ""; if (forReset) password = ""; error = ""; info = "We sent a code to ${email.trim()}." }
    fun submit() {
        error = ""
        val bad = SignInRules.problem(step, reset, email, password, handle, code)
        if (bad != null) { error = bad; return }
        val mail = email.trim()
        when {
            step == SignInStep.SIGN_IN -> signIn()
            step == SignInStep.SIGN_UP -> send("register", mapOf("email" to mail, "password" to password, "handle" to handle.trim())) { sent(false) }
            step == SignInStep.FORGOT -> send("forgot", mapOf("email" to mail)) { sent(true) }
            reset -> send("reset", mapOf("email" to mail, "code" to SignInRules.codeOf(code), "password" to password)) {
                step = SignInStep.SIGN_IN; reset = false; code = ""; error = ""; info = "Password changed. Sign in with the new one."
            }
            else -> send("verify", mapOf("email" to mail, "code" to SignInRules.codeOf(code))) { signIn() }
        }
    }
    val titles = mapOf(SignInStep.SIGN_IN to "Sign in", SignInStep.SIGN_UP to "Create an account", SignInStep.FORGOT to "Reset your password")
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        Text(if (step == SignInStep.CODE) (if (reset) "Choose a new password" else "Enter the code") else titles.getValue(step),
            color = c.text, style = MaterialTheme.typography.titleSmall)
        if (info.isNotEmpty()) Text(info, color = c.textDim, style = MaterialTheme.typography.bodyMedium)
        if (step != SignInStep.CODE) FlatField(email, { email = it }, "Email", Modifier.fillMaxWidth().semantics { contentType = ContentType.EmailAddress },
            keyboardType = KeyboardType.Email)
        if (step == SignInStep.SIGN_UP) FlatField(handle, { handle = it }, "Username", Modifier.fillMaxWidth().semantics { contentType = ContentType.NewUsername })
        if (step == SignInStep.CODE) FlatField(code, { code = it }, "Six digit code", Modifier.fillMaxWidth().semantics { contentType = ContentType.SmsOtpCode },
            keyboardType = KeyboardType.NumberPassword, mono = true)
        if (step == SignInStep.SIGN_IN || step == SignInStep.SIGN_UP || (step == SignInStep.CODE && reset)) {
            val newPassword = step != SignInStep.SIGN_IN
            FlatField(password, { password = it }, if (step == SignInStep.CODE) "New password" else "Password",
                Modifier.fillMaxWidth().semantics { contentType = if (newPassword) ContentType.NewPassword else ContentType.Password },
                masked = true, keyboardType = KeyboardType.Password)
        }
        if (error.isNotEmpty()) Text(error, color = c.bad, style = MaterialTheme.typography.bodyMedium)
        PrimaryButton(when {
            step == SignInStep.SIGN_IN -> "Sign in"
            step == SignInStep.SIGN_UP -> "Create account"
            step == SignInStep.FORGOT -> "Send code"
            reset -> "Save password"
            else -> "Confirm"
        }, ::submit, enabled = !busy)
        Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            fun link(text: String, onClick: () -> Unit) = @Composable {
                Text(text, color = c.accent, style = MaterialTheme.typography.bodyMedium,
                    modifier = Modifier.heightIn(min = 48.dp).wrapContentHeight(androidx.compose.ui.Alignment.CenterVertically).clickable(onClick = onClick))
            }
            when (step) {
                SignInStep.SIGN_IN -> {
                    link("Create an account") { go(SignInStep.SIGN_UP) }()
                    link("Forgot password") { go(SignInStep.FORGOT) }()
                    if (error.contains("not confirmed") && SignInRules.emailOk(email))
                        link("Send a new code") { send("resend", mapOf("email" to email.trim())) { sent(false) } }()
                }
                SignInStep.SIGN_UP, SignInStep.FORGOT -> link("I have an account") { go(SignInStep.SIGN_IN) }()
                SignInStep.CODE -> {
                    if (SignInRules.emailOk(email)) link("Send a new code") {
                        if (reset) send("forgot", mapOf("email" to email.trim())) { sent(true) } else send("resend", mapOf("email" to email.trim())) { sent(false) }
                    }()
                    link("Back") { go(SignInStep.SIGN_IN) }()
                }
            }
        }
    }
}

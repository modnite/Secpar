package com.pcremote

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.preference.PreferenceManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.*
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.lifecycleScope
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.pcremote.R
import com.pcremote.agent.agentPair
import com.pcremote.home.HomeActions
import com.pcremote.home.HomeController
import com.pcremote.home.HomeScreen
import com.pcremote.home.PrimaryAction
import com.pcremote.nav.Mode
import com.pcremote.nav.Routes
import com.pcremote.nav.exitScreen
import com.pcremote.onboarding.OnboardingScreen
import com.pcremote.onboarding.PairingDeps
import com.pcremote.onboarding.PairingModel
import com.pcremote.onboarding.PairingOverlay
import com.pcremote.onboarding.scanQr
import com.pcremote.ui.theme.MidnightTheme
import kotlinx.coroutines.launch
import kotlin.concurrent.thread

const val PREF_ONBOARDED = "pcremote_onboarded"
const val SETUP_GUIDE_URL = "https://github.com/modnite/Secpar#readme"

/** Modes with a screen in this build. */
val IMPLEMENTED_MODES = setOf(Mode.SCREEN, Mode.REMOTE, Mode.MEDIA, Mode.SLIDES, Mode.PAD, Mode.FILES)

class MainActivity : ComponentActivity() {
    var pipVisible by mutableStateOf(false)
        private set
    var onStreamHidden: (() -> Unit)? = null
    private var pipActive = false
    private var streamWindowActive = false
    private var pipAspect = android.util.Rational(16, 9)
    val supportsPip get() = packageManager.hasSystemFeature(android.content.pm.PackageManager.FEATURE_PICTURE_IN_PICTURE)

    /** Keep non-stream screens out of the previous landscape/immersive window state. */
    fun setStreamWindowActive(active: Boolean) {
        streamWindowActive = active
        refreshWindowInsets()
    }

    private fun refreshWindowInsets() {
        val decor = window.decorView
        decor.postOnAnimation {
            if (isDestroyed) return@postOnAnimation
            if (!streamWindowActive && !isInPictureInPictureMode) {
                androidx.core.view.WindowCompat.getInsetsController(window, decor).apply {
                    systemBarsBehavior = androidx.core.view.WindowInsetsControllerCompat.BEHAVIOR_DEFAULT
                    show(androidx.core.view.WindowInsetsCompat.Type.systemBars())
                }
            }
            // A handled configuration change retains the Compose view. Ask the
            // current window for fresh insets after its resize, rather than keeping
            // a previous landscape cutout/status-bar snapshot after background/PiP.
            androidx.core.view.ViewCompat.requestApplyInsets(decor)
        }
    }

    override fun onResume() {
        super.onResume()
        refreshWindowInsets()
    }

    override fun onConfigurationChanged(newConfig: android.content.res.Configuration) {
        super.onConfigurationChanged(newConfig)
        refreshWindowInsets()
    }

    fun configureStreamPip(active: Boolean, width: Int = 16, height: Int = 9) {
        pipActive = active && supportsPip
        if (width > 0 && height > 0) {
            val ratio = (width.toFloat() / height).coerceIn(1f / 2.39f, 2.39f)
            pipAspect = android.util.Rational((ratio * 1000).toInt(), 1000)
        }
        if (supportsPip) runCatching { setPictureInPictureParams(streamPipParams()) }
    }

    private fun streamPipParams(): android.app.PictureInPictureParams {
        val builder = android.app.PictureInPictureParams.Builder().setAspectRatio(pipAspect)
        if (Build.VERSION.SDK_INT >= 31) builder.setAutoEnterEnabled(pipActive)
        return builder.build()
    }

    fun enterStreamPip(): Boolean {
        if (!pipActive) return false
        desktopCapture?.setRequested(false)
        return runCatching { enterPictureInPictureMode(streamPipParams()) }.getOrDefault(false)
    }

    override fun onStop() {
        // PiP remains started. Dismissing it or hiding the stream must end remote control.
        if (pipActive && !isChangingConfigurations) endHiddenStream()
        super.onStop()
    }

    override fun onDestroy() {
        // A PiP task can be removed without another stop callback. Capture the
        // stream callback before the composition is disposed, and invoke once.
        if (isFinishing && !isChangingConfigurations) endHiddenStream()
        super.onDestroy()
    }

    private fun endHiddenStream() {
        val end=onStreamHidden ?: return
        onStreamHidden=null
        end()
    }

    override fun onUserLeaveHint() {
        super.onUserLeaveHint()
        if (Build.VERSION.SDK_INT < 31 && pipActive) enterStreamPip()
    }

    override fun onPictureInPictureModeChanged(isInPictureInPictureMode: Boolean, newConfig: android.content.res.Configuration) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig)
        pipVisible = isInPictureInPictureMode
        if (isInPictureInPictureMode) desktopCapture?.setRequested(false)
        else refreshWindowInsets()
    }

    var desktopCapture: com.pcremote.stream2.DesktopCapture? = null
    private var captureShortcutKey: Int? = null
    /** Set by the stream screen: Ctrl+Alt+Q / Ctrl+Alt+S from a hardware keyboard. */
    var streamChords: ((com.pcremote.stream2.StreamChord) -> Unit)? = null
    private var chordKey: Int? = null
    var desktopMotion: ((android.view.MotionEvent) -> Boolean)? = null
    override fun dispatchGenericMotionEvent(event: android.view.MotionEvent): Boolean =
        (desktopCapture?.let { it.forwarding && !it.freePointer } == true && desktopMotion?.invoke(event) == true) || super.dispatchGenericMotionEvent(event)

    override fun dispatchTouchEvent(event: android.view.MotionEvent): Boolean {
        if (desktopCapture?.let { it.forwarding && !it.freePointer } == true && event.isFromSource(android.view.InputDevice.SOURCE_MOUSE) && desktopMotion?.invoke(event) == true) return true
        return super.dispatchTouchEvent(event)
    }

    var desktopInput: com.pcremote.stream2.DesktopInput? = null

    override fun onWindowFocusChanged(hasFocus: Boolean) {
        super.onWindowFocusChanged(hasFocus)
        if (!hasFocus) desktopCapture?.lostFocus() else desktopCapture?.gainedFocus()
        if (hasFocus) refreshWindowInsets()
    }

    private lateinit var home: HomeController
    /** The paste-a-link dialog (phones without a usable camera). */
    private var manualOpen by mutableStateOf(false)
    private lateinit var pairing: PairingModel

    /** pcremote://pair… links (camera-app scans) start pairing straight away. */
    private fun handlePairLink(intent: Intent?) {
        if (intent?.getBooleanExtra("secpar_paste_pairing", false) == true) {
            intent.removeExtra("secpar_paste_pairing")
            manualOpen = true
        }
        val link = intent?.dataString?.takeIf { it.startsWith("secpar://pair?") || it.startsWith("pcremote://pair?") } ?: return
        intent.data = null
        lifecycleScope.launch { pairing.onScanned(link) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handlePairLink(intent)
    }

    @Suppress("DEPRECATION")
    private val prefs by lazy { PreferenceManager.getDefaultSharedPreferences(this) }

    override fun onCreate(savedInstanceState: Bundle?) {
        val settings = com.pcremote.settings.SettingsModel.get(this)
        val darkBars = settings.theme != com.pcremote.settings.ThemeChoice.LIGHT
        enableEdgeToEdge(
            statusBarStyle = if (darkBars) androidx.activity.SystemBarStyle.dark(android.graphics.Color.TRANSPARENT)
            else androidx.activity.SystemBarStyle.auto(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT),
            navigationBarStyle = if (darkBars) androidx.activity.SystemBarStyle.dark(android.graphics.Color.TRANSPARENT)
            else androidx.activity.SystemBarStyle.auto(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT),
        )
        super.onCreate(savedInstanceState)
        home = HomeController(applicationContext, lifecycleScope)
        pairing = PairingModel(PairingDeps(
            agentPair = { uri, device, pin -> agentPair(lifecycleScope, uri, device, pin) },
            save = { home.store.upsert(it); home.refreshStore() },
            device = Build.MODEL,
        ))
        handlePairLink(intent)
        com.pcremote.update.Updater.schedule(this)
        // transfer progress and update prompts are notifications
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED) {
            requestPermissions(arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), 1)
        }
        val start = Routes.start(if (home.store.all().isNotEmpty()) 1 else 0, prefs.getBoolean(PREF_ONBOARDED, false))

        setContent {
            var themeTick by remember { mutableIntStateOf(0) }
            val theme = remember(themeTick) { settings.theme }
            val dark = when (theme) {
                com.pcremote.settings.ThemeChoice.LIGHT -> false
                com.pcremote.settings.ThemeChoice.SYSTEM -> isSystemInDarkTheme()
                else -> true
            }
            MidnightTheme(dark = dark, dynamic = remember(themeTick) { settings.dynamicColor }) {
                val nav = rememberNavController()
                val scope = rememberCoroutineScope()
                fun finishOnboarding() {
                    prefs.edit().putBoolean(PREF_ONBOARDED, true).apply()
                    nav.navigate(Routes.HOME) { popUpTo(Routes.ONBOARDING) { inclusive = true } }
                }
                if (manualOpen) com.pcremote.onboarding.ManualPairDialog(
                    onPair = { link -> manualOpen = false; scope.launch { pairing.onScanned(link) } },
                    onDismiss = { manualOpen = false })
                NavHost(nav, start,
                    enterTransition = { slideInHorizontally { it / 6 } + fadeIn() },
                    exitTransition = { fadeOut() },
                    popEnterTransition = { fadeIn() },
                    popExitTransition = { slideOutHorizontally { it / 6 } + fadeOut() },
                ) {
                    composable(Routes.ONBOARDING) {
                        OnboardingScreen(pairing,
                            onDone = { pairing.reset(); finishOnboarding() },
                            onManual = ::manualPairing,
                            onSkip = ::finishOnboarding,
                            onAccount = { nav.navigate(Routes.ACCOUNT) })
                    }
                    composable(Routes.modePattern(Mode.SCREEN)) { e ->
                        com.pcremote.stream2.StreamScreen(modeId(e), onBack = { nav.exitScreen(e) })
                    }
                    composable(Routes.modePattern(Mode.REMOTE)) { e ->
                        com.pcremote.modes.RemoteScreen(modeId(e), onBack = { nav.popBackStack() })
                    }
                    composable(Routes.modePattern(Mode.MEDIA)) { e ->
                        com.pcremote.modes.MediaScreen(modeId(e), onBack = { nav.popBackStack() })
                    }
                    composable(Routes.modePattern(Mode.SLIDES)) { e ->
                        com.pcremote.modes.SlidesScreen(modeId(e), onBack = { nav.popBackStack() })
                    }
                    composable(Routes.modePattern(Mode.PAD)) { e ->
                        com.pcremote.modes.PadScreen(modeId(e), onBack = { nav.popBackStack() })
                    }
                    composable(Routes.modePattern(Mode.FILES)) { e ->
                        com.pcremote.modes.FilesScreen(modeId(e), onBack = { nav.popBackStack() })
                    }
                    composable(Routes.ACCOUNT) {
                        com.pcremote.account.AccountScreen(com.pcremote.account.AccountRuntime.get(this@MainActivity).controller,
                            onBack = { nav.popBackStack() }, onOpen = { id ->
                                prefs.edit().putBoolean(PREF_ONBOARDED, true).apply()
                                nav.navigate(Routes.HOME) { popUpTo(nav.graph.startDestinationId) { inclusive = true }; launchSingleTop = true }
                                nav.navigate(Routes.mode(id, Mode.SCREEN))
                            })
                    }
                    composable(Routes.SETTINGS) {
                        com.pcremote.settings.SettingsScreen(settings, onBack = { nav.popBackStack() },
                            onThemeChanged = { themeTick++; if (settings.theme == com.pcremote.settings.ThemeChoice.LIGHT) recreate() })
                    }
                    composable(Routes.HOME) {
                        val account = com.pcremote.account.AccountRuntime.get(this@MainActivity).controller
                        LifecycleResumeEffect(account) {
                            val poll = lifecycleScope.launch { while (true) { account.refresh(); kotlinx.coroutines.delay(30_000) } }
                            onPauseOrDispose { poll.cancel() }
                        }
                        val accountRevision by account.revision.collectAsState()
                        LifecycleResumeEffect(accountRevision) { home.start(); onPauseOrDispose { home.stop() } }
                        val cards by home.cards.collectAsState()
                        val thumbs by home.thumbs.collectAsState()
                        val pairState by pairing.state.collectAsState()
                        LaunchedEffect(pairState) {
                            if (pairState is com.pcremote.onboarding.PairingState.Done) { pairing.reset(); home.stop(); home.start() }
                        }
                        HomeScreen(cards, thumbs, IMPLEMENTED_MODES, HomeActions(
                            open = { card, mode -> nav.navigate(Routes.mode(card.id, mode)) },
                            primary = { card ->
                                when (card.primary) {
                                    PrimaryAction.OPEN -> {
                                        card.modes.firstOrNull { it in IMPLEMENTED_MODES }?.let { nav.navigate(Routes.mode(card.id, it)) }
                                    }
                                    PrimaryAction.WAKE -> wake(card)
                                    PrimaryAction.OFFLINE_HELP -> startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(SETUP_GUIDE_URL)))
                                    PrimaryAction.CHECKING -> Unit
                                }
                            },
                            addPc = { scanQr(this@MainActivity, onUnavailable = ::manualPairing) { raw -> scope.launch { pairing.onScanned(raw) } } },
                            settings = { nav.navigate(Routes.SETTINGS) },
                            account = { nav.navigate(Routes.ACCOUNT) },
                            refresh = { home.stop(); home.start() },
                            rename = { card, name -> home.rename(card.id, name) },
                            remove = { card -> home.remove(card) },
                            send = { card, m -> home.connection(card.id)?.send(m) },
                            connection = { card -> home.connection(card.id) },
                        ))
                        PairingOverlay(pairState,
                            onRetry = { pairing.reset(); scanQr(this@MainActivity, onUnavailable = ::manualPairing) { raw -> scope.launch { pairing.onScanned(raw) } } },
                            onDismiss = pairing::reset)
                    }
                }
            }
        }
    }

    private fun modeId(e: androidx.navigation.NavBackStackEntry) =
        java.net.URLDecoder.decode(e.arguments?.getString(Routes.ARG_ID).orEmpty(), "UTF-8")

    private fun manualPairing() { manualOpen = true }

    private fun wake(card: com.pcremote.home.PcCardUi) {
        thread {
            val ok = home.wake(card)
            val msg = when (ok) {
                null -> R.string.pcr_wol_no_mac
                true -> R.string.pcr_wol_sent
                false -> R.string.pcr_wol_failed
            }
            runOnUiThread { Toast.makeText(this, msg, Toast.LENGTH_LONG).show() }
        }
    }

    override fun dispatchKeyEvent(event: android.view.KeyEvent): Boolean {
        val chords = streamChords
        if (event.keyCode == chordKey) {
            if (event.action == android.view.KeyEvent.ACTION_UP) chordKey = null
            return true
        }
        if (chords != null && event.isFromSource(android.view.InputDevice.SOURCE_KEYBOARD) && event.device?.isVirtual == false) {
            val chord = com.pcremote.stream2.StreamChords.match(event.keyCode, event.isCtrlPressed, event.isAltPressed)
            if (chord != null) {
                if (event.action == android.view.KeyEvent.ACTION_DOWN && event.repeatCount == 0) { chordKey = event.keyCode; chords(chord) }
                return true
            }
        }
        if (event.keyCode == captureShortcutKey) {
            if (event.action == android.view.KeyEvent.ACTION_UP) captureShortcutKey = null
            return true
        }
        val capture = desktopCapture
        val chosen = com.pcremote.settings.SettingsModel.get(this).captureShortcut
        val isToggle = chosen.matches(event.keyCode, event.isCtrlPressed, event.isAltPressed, event.isShiftPressed, event.isMetaPressed) ||
            com.pcremote.stream2.CaptureShortcut.MOONLIGHT.matches(event.keyCode, event.isCtrlPressed, event.isAltPressed, event.isShiftPressed, event.isMetaPressed)
        if (capture != null && !capture.freePointer && isToggle) {
            if (event.action == android.view.KeyEvent.ACTION_DOWN && event.repeatCount == 0) {
                captureShortcutKey = event.keyCode
                capture.toggle()
            }
            return true
        }
        if (capture?.forwarding == true && desktopInput?.key(event) == true) return true
        val up = when (event.keyCode) {
            android.view.KeyEvent.KEYCODE_VOLUME_UP -> true
            android.view.KeyEvent.KEYCODE_VOLUME_DOWN -> false
            else -> null
        }
        if (up != null) {
            if (event.action == android.view.KeyEvent.ACTION_DOWN && com.pcremote.modes.VolumeKeys.onVolumeKey(up, event.repeatCount > 0)) return true
            if (event.action == android.view.KeyEvent.ACTION_UP && com.pcremote.modes.VolumeKeys.active) return true
        }
        return super.dispatchKeyEvent(event)
    }

}

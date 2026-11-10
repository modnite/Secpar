package com.pcremote.settings

import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.DpSize
import androidx.compose.ui.unit.dp
import com.pcremote.R
import com.pcremote.home.RemoteWake
import com.pcremote.home.WakeMethod
import com.pcremote.stream2.STREAM_SIZES
import com.pcremote.stream2.sizeLabel
import com.pcremote.ui.components.*
import com.pcremote.ui.theme.LocalMidnight
import com.pcremote.ui.theme.MidnightShapes
import com.pcremote.ui.theme.rememberHaptics

private const val REPO_URL = "https://github.com/modnite/Secpar"
const val HELP_URL = "https://secpar.win/help/"

/** What the Video dropdowns offer, as the text shown. */
object SettingsOptions {
    val resolutions: List<Pair<Int, Int>> = STREAM_SIZES
    val resolutionLabels: List<String> = STREAM_SIZES.map { (w, h) -> sizeLabel(w, h) }
    val frameRateLabels: List<String> = SettingsModel.FRAME_RATES.map { "$it fps" }
    val bitrateLabels: List<String> = SettingsModel.BITRATES_MBPS.map { "$it Mbps" }
}

/** The Settings tab: a page title and one card per group. */
@Composable
fun SettingsPage(model: SettingsModel, onThemeChanged: () -> Unit, modifier: Modifier = Modifier) {
    var theme by remember { mutableStateOf(model.theme) }
    var haptics by remember { mutableStateOf(model.haptics) }
    var natural by remember { mutableStateOf(model.naturalScroll) }
    var pointer by remember { mutableFloatStateOf(model.pointerSpeed) }
    var gyro by remember { mutableFloatStateOf(model.gyroSensitivity) }
    var resolution by remember { mutableStateOf(model.resolution) }
    var fps by remember { mutableIntStateOf(model.frameRate) }
    var bitrate by remember { mutableIntStateOf(model.bitrateMbps) }
    var audio by remember { mutableStateOf(model.playHostAudio) }
    var clipboard by remember { mutableStateOf(model.shareClipboard) }
    val ctx = LocalContext.current

    Column(modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(start = 20.dp, end = 20.dp, top = 8.dp, bottom = 24.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp)) {


        Group(R.string.pcr_set_video) {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FlatDropdown(stringResource(R.string.pcr_resolution), sizeLabel(resolution.first, resolution.second), SettingsOptions.resolutionLabels,
                    { i -> resolution = SettingsOptions.resolutions[i]; model.resolution = resolution }, Modifier.weight(1f))
                FlatDropdown(stringResource(R.string.pcr_frame_rate), "$fps fps", SettingsOptions.frameRateLabels,
                    { i -> fps = SettingsModel.FRAME_RATES[i]; model.frameRate = fps }, Modifier.weight(1f))
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FlatDropdown(stringResource(R.string.pcr_bitrate_limit), "$bitrate Mbps", SettingsOptions.bitrateLabels,
                    { i -> bitrate = SettingsModel.BITRATES_MBPS[i]; model.bitrateMbps = bitrate }, Modifier.weight(1f))
                Spacer(Modifier.weight(1f))
            }
            }
        }
        Group(R.string.pcr_set_audio) {
            FlatCheckbox(audio, stringResource(R.string.pcr_play_host_audio), { audio = it; model.playHostAudio = it })
        }
        Group(R.string.pcr_set_clipboard) {
            FlatCheckbox(clipboard, stringResource(R.string.pcr_share_clipboard), { clipboard = it; model.shareClipboard = it })
        }
        Group(R.string.pcr_set_controls) {
            FlatCheckbox(haptics, stringResource(R.string.pcr_haptics), { haptics = it; model.haptics = it })
            FlatCheckbox(natural, stringResource(R.string.pcr_natural_scroll), { natural = it; model.naturalScroll = it })
            SliderRow(R.string.pcr_pointer_speed, pointer, 0.5f..3f) { pointer = it; model.pointerSpeed = it }
            SliderRow(R.string.pcr_gyro_sensitivity, gyro, 0.25f..3f) { gyro = it; model.gyroSensitivity = it }
        }
        Group(R.string.pcr_set_appearance) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                listOf(ThemeChoice.SYSTEM to R.string.pcr_theme_system, ThemeChoice.DARK to R.string.pcr_theme_dark,
                    ThemeChoice.LIGHT to R.string.pcr_theme_light).forEach { (t, label) ->
                    Segment(stringResource(label), selected = theme == t, Modifier.weight(1f)) {
                        theme = t; model.theme = t; onThemeChanged()
                    }
                }
            }
        }
        WakeGroup()
        UnlockGroup()
        Group(R.string.pcr_set_about) {
            LinkRow(R.string.pcr_help) { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(HELP_URL))) }
            LinkRow(R.string.pcr_source) { ctx.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(REPO_URL))) }
        }
    }
}

/** Wake from the phone: on the network, through an HTTPS link, or through Home Assistant. */
@Composable
private fun WakeGroup() {
    val c = LocalMidnight.current
    val ctx = LocalContext.current
    val ha = remember { com.pcremote.home.HomeAssistant.get(ctx) }
    val remote = remember { RemoteWake.get(ctx) }
    var method by remember { mutableStateOf(remote.method) }
    var url by remember(method) { mutableStateOf(if (method == WakeMethod.HOME_ASSISTANT) ha.url else remote.url) }
    var token by remember(method) { mutableStateOf(if (method == WakeMethod.HOME_ASSISTANT) ha.token else remote.token) }
    var saved by remember(method) { mutableStateOf(false) }
    var invalid by remember { mutableStateOf(false) }
    Group(R.string.pcr_set_wake) {
        WakeMethod.entries.forEach { option ->
            Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable { method = option; remote.method = option; invalid = false },
                verticalAlignment = Alignment.CenterVertically) {
                RadioButton(selected = option == method, onClick = null)
                Spacer(Modifier.width(10.dp))
                Text(stringResource(when (option) {
                    WakeMethod.LOCAL -> R.string.secpar_wake_local
                    WakeMethod.WEBHOOK -> R.string.secpar_wake_webhook
                    WakeMethod.HOME_ASSISTANT -> R.string.secpar_wake_ha
                }), color = c.text, style = MaterialTheme.typography.bodyMedium)
            }
        }
        if (method != WakeMethod.LOCAL) {
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                FlatField(url, { url = it; saved = false; invalid = false }, stringResource(if (method == WakeMethod.WEBHOOK) R.string.secpar_wake_link else R.string.pcr_ha_url),
                    Modifier.fillMaxWidth(), masked = true, isError = invalid)
                if (invalid) Text(stringResource(R.string.secpar_wake_https), color = c.bad, style = MaterialTheme.typography.bodySmall)
                FlatField(token, { token = it; saved = false }, stringResource(R.string.secpar_wake_token), Modifier.fillMaxWidth(), masked = true)
                GhostButton(stringResource(if (saved) R.string.pcr_saved else R.string.pcr_save), {
                    if (method == WakeMethod.WEBHOOK) {
                        invalid = !RemoteWake.validUrl(url)
                        if (!invalid) { remote.url = url; remote.token = token; saved = true }
                    } else { ha.url = url; ha.token = token; saved = true }
                }, icon = if (saved) R.drawable.lucide_check else null)
            }
        }
    }
}

/** PINs kept behind the fingerprint, one per computer. */
@Composable
private fun UnlockGroup() {
    val c = LocalMidnight.current
    val ctx = LocalContext.current
    val pcs = remember { com.pcremote.agent.PcStore.get(ctx).all() }
    var tick by remember { mutableIntStateOf(0) }
    Group(R.string.pcr_set_unlock) {
        key(tick) {
            val saved = pcs.filter { com.pcremote.agent.PinVault.has(ctx, it.id) }
            if (saved.isEmpty()) Text(stringResource(R.string.pcr_unlock_none), color = c.textDim, style = MaterialTheme.typography.bodyMedium)
            saved.forEach { pc ->
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(pc.name, color = c.text, style = MaterialTheme.typography.bodyMedium, maxLines = 1,
                        overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    GhostButton(stringResource(R.string.pcr_forget), { com.pcremote.agent.PinVault.forget(ctx, pc.id); tick++ }, tint = c.bad, compact = true)
                }
            }
        }
    }
}

/** Settings card: radius 7, margin 10, title 14 strong inside at the top. */
@Composable
private fun Group(title: Int, content: @Composable ColumnScope.() -> Unit) {
    val c = LocalMidnight.current
    Column(Modifier.fillMaxWidth().clip(MidnightShapes.card).background(c.surface).padding(10.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(stringResource(title), color = c.text, style = MaterialTheme.typography.titleSmall)
        content()
    }
}

@Composable
private fun Segment(text: String, selected: Boolean, modifier: Modifier, onClick: () -> Unit) {
    val c = LocalMidnight.current
    val h = rememberHaptics()
    Box(modifier.minimumInteractiveComponentSize().height(32.dp).clip(MidnightShapes.control).background(if (selected) c.accent else c.surfaceHigh)
        .clickable { h.tick(); onClick() }, contentAlignment = Alignment.Center) {
        Text(text, color = if (selected) c.onAccent else c.text, style = MaterialTheme.typography.labelLarge)
    }
}

@Composable
private fun LinkRow(title: Int, onClick: () -> Unit) {
    val c = LocalMidnight.current
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable(onClick = onClick), verticalAlignment = Alignment.CenterVertically) {
        Text(stringResource(title), color = c.text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
        Icon(painterResource(R.drawable.lucide_chevron_right), null, tint = c.textDim, modifier = Modifier.size(18.dp))
    }
}

/** Label and value (12 muted) over a flat slider: 4dp track, 16dp thumb, no halo. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SliderRow(title: Int, value: Float, range: ClosedFloatingPointRange<Float>, onChange: (Float) -> Unit) {
    val c = LocalMidnight.current
    val source = remember { MutableInteractionSource() }
    Column {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(title), color = c.text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
            Text("%.1f×".format(value), color = c.textDim, style = MaterialTheme.typography.bodySmall)
        }
        val colors = SliderDefaults.colors(thumbColor = c.accent, activeTrackColor = c.accent, inactiveTrackColor = c.surfaceHigh)
        Slider(value, onChange, valueRange = range, interactionSource = source, colors = colors,
            thumb = { SliderDefaults.Thumb(source, colors = colors, thumbSize = DpSize(16.dp, 16.dp)) },
            track = { state -> SliderDefaults.Track(state, Modifier.height(4.dp), colors = colors, drawStopIndicator = null, thumbTrackGapSize = 0.dp) })
    }
}

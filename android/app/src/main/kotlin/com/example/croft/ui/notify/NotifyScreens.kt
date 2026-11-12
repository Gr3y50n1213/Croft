// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.notify

import android.Manifest
import android.content.Context
import android.content.ContextWrapper
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.LifecycleOwner
import com.example.croft.R
import com.example.croft.notify.AndroidNotifier
import com.example.croft.notify.CheckAccount
import com.example.croft.notify.CheckFrequency
import com.example.croft.notify.NotifyAction
import com.example.croft.notify.NotifySettings
import com.example.croft.ui.appearance.SectionLabel
import com.example.croft.ui.appearance.SettingsFrame
import com.example.croft.ui.components.ButtonKind
import com.example.croft.ui.components.CroftButton
import com.example.croft.ui.components.CroftCheckboxRow
import com.example.croft.ui.components.CroftRadioRow
import com.example.croft.ui.components.CroftSwitchRow
import kotlinx.coroutines.launch

/** Test tags of S07 and S46. */
object NotifyTags {
    const val INTRO = "S07"
    const val INTRO_TURN_ON = "S07-turn-on"
    const val INTRO_SKIP = "S07-skip"
    const val INTRO_DONE = "S07-done"
    const val INTRO_TEST = "S07-test"
    const val PERIODIC = "S07-periodic"
    const val DENIED = "notify-denied"
    const val DENIED_SETTINGS = "notify-denied-settings"
    const val SETTINGS = "S46"
    const val MASTER = "S46-master"
    const val FREQUENCY = "S46-frequency"
    const val BATTERY_NOT_LOW = "S46-battery-not-low"
    const val LOCK_SCREEN = "S46-lock-screen"
    const val SEND_TEST = "S46-test"
    const val BATTERY_SETTINGS = "S46-battery-settings"
    fun account(id: Long) = "S46-account-$id"
    fun frequency(f: CheckFrequency) = "S46-frequency-${f.key}"
    fun action(a: NotifyAction) = "S46-action-${a.key}"
}

/** Where S07 is: asking, on, or refused (a calm banner with a Settings link; never a second prompt). */
enum class IntroState { ASK, ON, DENIED }

/** The notification permission: asked only on Android 13 and later, and only from S07. */
object NotifyPermission {
    /** Whether S07's "Turn on" must show the system prompt: Android 13 and later, permission not yet granted. Below 13 there is no prompt. */
    fun needsPrompt(context: Context, sdk: Int = Build.VERSION.SDK_INT): Boolean =
        sdk >= Build.VERSION_CODES.TIRAMISU && ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED

    /** Whether a notification can show now. */
    fun canPost(context: Context): Boolean = NotificationManagerCompat.from(context).areNotificationsEnabled()

    /** The system's notification settings for Croft. */
    fun openSettings(context: Context) {
        val intent = Intent(Settings.ACTION_APP_NOTIFICATION_SETTINGS)
            .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try { context.startActivity(intent) } catch (e: Exception) { /* no settings screen on this device */ }
    }

    /**
     * The system's battery optimisation list, where the person can let Croft run in the background. Croft never asks for
     * `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` (Play restricts it): it only links here.
     */
    fun openBatterySettings(context: Context) {
        val intent = Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        try { context.startActivity(intent) } catch (e: Exception) { /* no such screen on this device */ }
    }
}

/**
 * S07, shown once after the first account: why notifications, how Croft checks, then the system prompt (Android 13 and later only, at that
 * moment). Only Periodic sync is offered: push methods are not shown until they exist (DECISIONS: a disabled "coming later" option is
 * avoided on every platform). Skipping is fine; the app works without notifications.
 */
@Composable
fun NotifyIntroScreen(
    state: IntroState,
    onTurnOn: () -> Unit,
    onSendTest: () -> Unit,
    onSkip: () -> Unit,
    onDone: () -> Unit,
    onOpenSettings: () -> Unit,
    modifier: Modifier = Modifier,
    note: String? = null,
) {
    SettingsFrame(title = stringResource(R.string.notify_intro_title), onBack = onSkip, modifier = modifier.testTag(NotifyTags.INTRO)) {
        Text(stringResource(R.string.notify_intro_body), style = MaterialTheme.typography.bodyLarge)
        SectionLabel(stringResource(R.string.notify_intro_method))
        Column(Modifier.selectableGroup()) {
            CroftRadioRow(
                title = stringResource(R.string.notify_intro_periodic),
                hint = stringResource(R.string.notify_intro_periodic_hint),
                selected = true,
                onSelect = {},
                modifier = Modifier.testTag(NotifyTags.PERIODIC),
            )
        }
        when (state) {
            IntroState.ASK -> {
                CroftButton(stringResource(R.string.notify_intro_turn_on), onTurnOn, Modifier.fillMaxWidth().testTag(NotifyTags.INTRO_TURN_ON))
                CroftButton(stringResource(R.string.notify_intro_skip), onSkip, Modifier.fillMaxWidth().testTag(NotifyTags.INTRO_SKIP), kind = ButtonKind.TEXT)
            }
            IntroState.ON -> {
                Text(stringResource(R.string.notify_intro_on), style = MaterialTheme.typography.bodyLarge, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
                CroftButton(stringResource(R.string.notify_send_test), onSendTest, Modifier.fillMaxWidth().testTag(NotifyTags.INTRO_TEST), kind = ButtonKind.OUTLINED)
                CroftButton(stringResource(R.string.notify_intro_done), onDone, Modifier.fillMaxWidth().testTag(NotifyTags.INTRO_DONE))
            }
            IntroState.DENIED -> {
                DeniedBanner(onOpenSettings)
                CroftButton(stringResource(R.string.notify_intro_done), onDone, Modifier.fillMaxWidth().testTag(NotifyTags.INTRO_DONE))
            }
        }
        if (note != null) Text(note, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
    }
}

/** The calm banner shown while notifications cannot show: words and a Settings link, no prompt. */
@Composable
fun DeniedBanner(onOpenSettings: () -> Unit, modifier: Modifier = Modifier) {
    Surface(
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        shape = MaterialTheme.shapes.medium,
        modifier = modifier.fillMaxWidth().testTag(NotifyTags.DENIED),
    ) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.notify_denied_banner), style = MaterialTheme.typography.bodyMedium, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
            Row {
                CroftButton(stringResource(R.string.notify_denied_action), onOpenSettings, Modifier.testTag(NotifyTags.DENIED_SETTINGS), kind = ButtonKind.TONAL)
            }
        }
    }
}

/**
 * S07 with its parts: the store and scheduling are the caller's ([onChange]); the permission prompt is [requestPermission] (the system's,
 * through the activity result API; a fake in tests). [onFinished] goes on to the Inbox.
 */
@Composable
fun NotifyIntroHost(
    settings: NotifySettings,
    onChange: suspend (NotifySettings) -> Unit,
    accounts: suspend () -> List<CheckAccount>,
    onFinished: () -> Unit,
    requestPermission: ((onResult: (Boolean) -> Unit) -> Unit)? = null,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var state by remember { mutableStateOf(IntroState.ASK) }
    var note by remember { mutableStateOf<String?>(null) }
    val system = systemPermissionRequest()
    val ask = requestPermission ?: system
    val finish: () -> Unit = { scope.launch { onChange(settings.copy(introDone = true)); onFinished() } }
    // Back from the system settings with notifications turned on: the banner goes.
    val resumed = rememberResumeCount()
    LaunchedEffect(resumed) { if (state == IntroState.DENIED && NotifyPermission.canPost(context)) state = IntroState.ON }
    BackHandler(onBack = finish)
    NotifyIntroScreen(
        state = state,
        onTurnOn = {
            scope.launch {
                onChange(settings.copy(enabled = true))
                if (NotifyPermission.needsPrompt(context)) {
                    ask { granted -> state = if (granted && NotifyPermission.canPost(context)) IntroState.ON else IntroState.DENIED }
                } else {
                    state = if (NotifyPermission.canPost(context)) IntroState.ON else IntroState.DENIED
                }
            }
        },
        onSendTest = { scope.launch { note = sendTest(context, settings, accounts()) } },
        onSkip = finish,
        onDone = finish,
        onOpenSettings = { NotifyPermission.openSettings(context) },
        note = note,
    )
}

@Composable
private fun systemPermissionRequest(): (onResult: (Boolean) -> Unit) -> Unit {
    var pending by remember { mutableStateOf<((Boolean) -> Unit)?>(null) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted -> pending?.invoke(granted); pending = null }
    return { onResult ->
        pending = onResult
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) launcher.launch(Manifest.permission.POST_NOTIFICATIONS) else onResult(true)
    }
}

/** "Send test": a real notification through the same code path, for the first account that can notify. */
private fun sendTest(context: Context, settings: NotifySettings, accounts: List<CheckAccount>): String {
    val account = accounts.firstOrNull { !it.isDemo } ?: return context.getString(R.string.notify_no_accounts)
    val ok = AndroidNotifier(context).postTest(account, settings)
    return context.getString(if (ok) R.string.notify_test_sent else R.string.notify_test_failed)
}

/**
 * S46 Notifications (minimal, T20): the master switch, a switch per account, how often Croft checks (and only when the battery is not
 * low), the lock-screen privacy option, the buttons (up to three), Send test and a link to the system's battery settings. While
 * notifications cannot show, the calm banner with a Settings link is on top. Quiet hours and push health come in phase 2.
 */
@Composable
fun NotificationSettingsScreen(
    settings: NotifySettings,
    accounts: List<CheckAccount>,
    canPost: Boolean,
    onChange: (NotifySettings) -> Unit,
    onSendTest: () -> Unit,
    onOpenSettings: () -> Unit,
    onOpenBatterySettings: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    note: String? = null,
) {
    SettingsFrame(title = stringResource(R.string.notify_settings_title), onBack = onBack, modifier = modifier.testTag(NotifyTags.SETTINGS)) {
        if (!canPost) DeniedBanner(onOpenSettings)
        CroftSwitchRow(
            title = stringResource(R.string.notify_master),
            checked = settings.enabled,
            onCheckedChange = { onChange(settings.copy(enabled = it)) },
            modifier = Modifier.testTag(NotifyTags.MASTER),
        )
        val real = accounts.filter { !it.isDemo }
        SectionLabel(stringResource(R.string.notify_accounts))
        if (real.isEmpty()) {
            Text(stringResource(R.string.notify_no_accounts), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        real.forEach { a ->
            CroftSwitchRow(
                title = a.label,
                checked = a.id !in settings.mutedAccounts,
                enabled = settings.enabled,
                onCheckedChange = { on -> onChange(settings.copy(mutedAccounts = if (on) settings.mutedAccounts - a.id else settings.mutedAccounts + a.id)) },
                modifier = Modifier.testTag(NotifyTags.account(a.id)),
            )
        }
        SectionLabel(stringResource(R.string.notify_frequency))
        // Five choices: a radio list at every width (a five-part segmented row would cut its labels on a phone).
        Column(Modifier.selectableGroup().testTag(NotifyTags.FREQUENCY)) {
            CheckFrequency.entries.forEach { f ->
                CroftRadioRow(
                    title = stringResource(frequencyLabel(f)),
                    selected = settings.frequency == f,
                    onSelect = { onChange(settings.copy(frequency = f)) },
                    modifier = Modifier.testTag(NotifyTags.frequency(f)),
                )
            }
        }
        Text(stringResource(R.string.notify_frequency_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        CroftSwitchRow(
            title = stringResource(R.string.notify_battery_not_low),
            checked = settings.batteryNotLow,
            enabled = settings.frequency != CheckFrequency.MANUAL,
            onCheckedChange = { onChange(settings.copy(batteryNotLow = it)) },
            modifier = Modifier.testTag(NotifyTags.BATTERY_NOT_LOW),
        )
        CroftSwitchRow(
            title = stringResource(R.string.notify_lock_screen),
            hint = stringResource(R.string.notify_lock_screen_hint),
            checked = settings.hideOnLockScreen,
            enabled = settings.enabled,
            onCheckedChange = { onChange(settings.copy(hideOnLockScreen = it)) },
            modifier = Modifier.testTag(NotifyTags.LOCK_SCREEN),
        )
        SectionLabel(stringResource(R.string.notify_actions))
        Text(stringResource(R.string.notify_actions_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        val full = settings.actions.size >= NotifySettings.MAX_ACTIONS
        NotifyAction.entries.forEach { action ->
            val on = action in settings.actions
            CroftCheckboxRow(
                title = stringResource(AndroidNotifier.actionLabel(action)),
                state = ToggleableState(on),
                enabled = settings.enabled && (on || !full),
                onToggle = { onChange(settings.withAction(action, it)) },
                modifier = Modifier.testTag(NotifyTags.action(action)),
            )
        }
        CroftButton(
            stringResource(R.string.notify_send_test),
            onSendTest,
            Modifier.fillMaxWidth().padding(top = 8.dp).testTag(NotifyTags.SEND_TEST),
            kind = ButtonKind.OUTLINED,
            enabled = canPost && settings.enabled && real.isNotEmpty(),
        )
        if (note != null) Text(note, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
        SectionLabel(stringResource(R.string.notify_battery_settings), Modifier.semantics { heading() })
        Text(stringResource(R.string.notify_battery_settings_hint), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        CroftButton(stringResource(R.string.notify_battery_settings), onOpenBatterySettings, Modifier.testTag(NotifyTags.BATTERY_SETTINGS), kind = ButtonKind.TONAL)
        Text(stringResource(R.string.notify_settings_later), style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp))
    }
}

/**
 * S46 with its parts: reads the accounts, re-checks whether notifications can show each time it is drawn, and sends the test.
 * [onChange] stores the settings and reschedules the work.
 */
@Composable
fun NotificationSettingsHost(
    settings: NotifySettings,
    onChange: (NotifySettings) -> Unit,
    accounts: suspend () -> List<CheckAccount>,
    onBack: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var list by remember { mutableStateOf<List<CheckAccount>>(emptyList()) }
    var note by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) { list = runCatching { accounts() }.getOrDefault(emptyList()) }
    NotificationSettingsScreen(
        settings = settings,
        accounts = list,
        canPost = remember(rememberResumeCount()) { NotifyPermission.canPost(context) },
        onChange = onChange,
        onSendTest = { scope.launch { note = sendTest(context, settings, list) } },
        onOpenSettings = { NotifyPermission.openSettings(context) },
        onOpenBatterySettings = { NotifyPermission.openBatterySettings(context) },
        onBack = onBack,
        note = note,
    )
}

private fun frequencyLabel(f: CheckFrequency): Int = when (f) {
    CheckFrequency.MIN_15 -> R.string.notify_freq_15m
    CheckFrequency.MIN_30 -> R.string.notify_freq_30m
    CheckFrequency.HOUR_1 -> R.string.notify_freq_1h
    CheckFrequency.HOUR_2 -> R.string.notify_freq_2h
    CheckFrequency.MANUAL -> R.string.notify_freq_manual
}

/** Counts the times the screen's activity came back to the front (from the system settings, for example). */
@Composable
private fun rememberResumeCount(): Int {
    val context = LocalContext.current
    var count by remember { mutableIntStateOf(0) }
    DisposableEffect(context) {
        val owner = context.lifecycleOwner()
        val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_RESUME) count++ }
        owner?.lifecycle?.addObserver(observer)
        onDispose { owner?.lifecycle?.removeObserver(observer) }
    }
    return count
}

private tailrec fun Context.lifecycleOwner(): LifecycleOwner? = when (this) {
    is LifecycleOwner -> this
    is ContextWrapper -> baseContext.lifecycleOwner()
    else -> null
}

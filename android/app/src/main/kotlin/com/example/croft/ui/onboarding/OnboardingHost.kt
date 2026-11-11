// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package com.example.croft.ui.onboarding

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.LayoutDirection
import com.example.croft.mail.MailSession
import com.example.croft.mail.onboarding.EmailInput
import com.example.croft.mail.onboarding.FirstSyncSource
import com.example.croft.mail.onboarding.FirstSyncStatus
import com.example.croft.mail.onboarding.OnboardingBackend
import com.example.croft.mail.onboarding.OnboardingFlow
import com.example.croft.mail.onboarding.Step
import com.example.croft.theme.LocalMotionLevel
import com.example.croft.ui.components.sharedAxisX
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.delay

/**
 * What the sign-in screens need from the app: the [backend] over the real core and secret store, the [firstSync] progress, and
 * [openSession], which makes the Inbox's session over the core once an account exists (until then the app shows the sample
 * mailbox). Tests give scripted versions of the first two.
 */
class OnboardingEnvironment(
    val backend: OnboardingBackend,
    val firstSync: FirstSyncSource,
    val openSession: suspend () -> MailSession,
)

/**
 * The sign-in flow S01 to S05 and S09, hosted in the app's navigation. Back goes one step back (through the flow's own stack)
 * and, on S01, is left to the system, which leaves the app. S09 has no Back arrow: Back there is "Continue in background".
 *
 * The flow survives rotation (its path, address and what was found are saved; **never the password**). [onFinished] is called
 * once, when the first sync is done or the person continues, with the new account's id; the caller then shows the Inbox.
 * [onTrySampleMail] is the one hook for "Try with sample mail" (see `onTrySampleMail` in the navigation).
 */
@Composable
fun OnboardingHost(
    environment: OnboardingEnvironment,
    onTrySampleMail: () -> Unit,
    onFinished: (accountId: Long) -> Unit,
    modifier: Modifier = Modifier,
    showCompanion: Boolean = OnboardingFlags.COMPANION_ENTRY,
) {
    val scope = rememberCoroutineScope()
    val flow = rememberSaveable(
        saver = Saver<OnboardingFlow, ArrayList<String>>(save = { it.snapshot() }, restore = { OnboardingFlow(environment.backend, scope, it) }),
    ) { OnboardingFlow(environment.backend, scope) }
    DisposableEffect(flow) { onDispose { flow.close() } }
    val state by flow.state.collectAsState()

    val finish = { state.addedAccount?.let(onFinished) ?: Unit }
    // S01: no handler, so the system leaves the app. S09: Back is Continue in background. Elsewhere: one step back.
    BackHandler(enabled = state.step != Step.WELCOME) { if (state.step == Step.FIRST_SYNC) finish() else flow.back() }

    val level = LocalMotionLevel.current
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    AnimatedContent(
        targetState = state.step, modifier = modifier, label = "onboarding-step",
        transitionSpec = { sharedAxisX(level, forward = targetState.ordinal > initialState.ordinal, rtl = rtl) },
    ) { step ->
        when (step) {
            Step.WELCOME -> WelcomeScreen(onAddAccount = flow::openAddAccount, onTrySampleMail = onTrySampleMail, showCompanion = showCompanion)
            Step.ADD_ACCOUNT -> AddAccountScreen(
                email = state.email, rejected = state.emailRejected, onEmailChange = flow::setEmail, onShortcut = flow::applyShortcut,
                onContinue = flow::continueWithEmail, onBack = { flow.back() },
            )
            Step.FINDING -> FindingScreen(
                email = state.email, probes = state.probes, lookup = state.lookup, onRetry = flow::retryLookup,
                onEnterManually = flow::openServerSettings, onBack = { flow.back() },
            )
            Step.SIGN_IN -> SignInScreen(
                state = state, onSignIn = flow::signIn, onServerSettings = flow::openServerSettings, onTrust = flow::trustServer,
                onCancelTrust = flow::cancelTrust, onPasswordEdited = flow::dismissFailure, onBack = { flow.back() },
            )
            Step.SERVER_SETTINGS -> ServerSettingsScreen(
                state = state, onFormChange = flow::updateForm, onTest = flow::testConnection, onAdd = flow::addFromSettings,
                onTrust = flow::trustServer, onCancelTrust = flow::cancelTrust, onBack = { flow.back() },
            )
            Step.FIRST_SYNC -> {
                var status by remember(state.addedAccount) { mutableStateOf<FirstSyncStatus?>(null) }
                val account = state.addedAccount
                LaunchedEffect(account) {
                    if (account == null) return@LaunchedEffect
                    while (true) {
                        val now = try {
                            environment.firstSync.status(account)
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            null
                        }
                        if (now != null) status = now
                        // Done: the Inbox has its first messages. (A failed account stays here until the person continues.)
                        if (now?.done == true) { onFinished(account); return@LaunchedEffect }
                        delay(FirstSyncPollMs)
                    }
                }
                FirstSyncScreen(accountName = EmailInput.localPartOf(state.email), status = status, onContinue = { finish() })
            }
        }
    }
}

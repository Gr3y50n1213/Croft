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
    /** T21: "Use your own client" on S04 for Gmail and Outlook. None: S04 offers only the password (tests of other parts). */
    val oauth: com.example.croft.mail.onboarding.OAuthBackend? = null,
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
    /** The first screen: S01, or S02 when Accounts (S44) adds another account. */
    startAt: Step = Step.WELCOME,
    /** Back on the first screen when it is not S01 (Accounts): leaves the flow. On S01 Back is left to the system. */
    onExit: () -> Unit = {},
    /** S01 says the sample mailbox could not be opened. */
    sampleProblem: Boolean = false,
    /** S01's privacy policy link (a view intent); no link when null. */
    onPrivacyPolicy: (() -> Unit)? = null,
    /**
     * Signing in again (the Needs sign-in banner): the account's address, looked up at once, so the flow goes straight on to S04 for it
     * (S04 offers the own-client sign-in for Gmail and Outlook). Adding an address that exists with the same servers is the core's
     * "sign in again": the account keeps its id and mail and its paused changes go on.
     */
    signInAgainEmail: String? = null,
) {
    val scope = rememberCoroutineScope()
    val flow = rememberSaveable(
        saver = Saver<OnboardingFlow, ArrayList<String>>(save = { it.snapshot() }, restore = { OnboardingFlow(environment.backend, scope, it) }),
    ) { OnboardingFlow(environment.backend, scope, startAt = startAt) }
    DisposableEffect(flow) { onDispose { flow.close() } }
    val state by flow.state.collectAsState()
    // Once per flow (not again after a rotation, which restores the flow where it was).
    var prefilled by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(signInAgainEmail) {
        if (!prefilled && signInAgainEmail != null && state.step == Step.ADD_ACCOUNT) {
            prefilled = true
            flow.setEmail(signInAgainEmail)
            flow.continueWithEmail()
        }
    }

    // The first sync ending, Continue and Back can all say "finished" at about the same moment: only the first counts, so the
    // app builds the Inbox's session once.
    val fired = remember { java.util.concurrent.atomic.AtomicBoolean(false) }
    val fire: (Long) -> Unit = { id -> if (fired.compareAndSet(false, true)) onFinished(id) }
    val finish = { state.addedAccount?.let(fire) ?: Unit }
    // S01: no handler, so the system leaves the app. S09: Back is Continue in background. Elsewhere: one step back.
    BackHandler(enabled = state.step != Step.WELCOME) { if (state.step == Step.FIRST_SYNC) finish() else if (!flow.back()) onExit() }

    val level = LocalMotionLevel.current
    val rtl = LocalLayoutDirection.current == LayoutDirection.Rtl
    AnimatedContent(
        targetState = state.step, modifier = modifier, label = "onboarding-step",
        transitionSpec = { sharedAxisX(level, forward = targetState.ordinal > initialState.ordinal, rtl = rtl) },
    ) { step ->
        when (step) {
            Step.WELCOME -> WelcomeScreen(onAddAccount = flow::openAddAccount, onTrySampleMail = onTrySampleMail, showCompanion = showCompanion, sampleProblem = sampleProblem, onPrivacyPolicy = onPrivacyPolicy)
            Step.ADD_ACCOUNT -> AddAccountScreen(
                email = state.email, rejected = state.emailRejected, onEmailChange = flow::setEmail, onShortcut = flow::applyShortcut,
                onContinue = flow::continueWithEmail, onBack = { if (!flow.back()) onExit() },
            )
            Step.FINDING -> FindingScreen(
                email = state.email, probes = state.probes, lookup = state.lookup, onRetry = flow::retryLookup,
                onEnterManually = flow::openServerSettings, onBack = { flow.back() },
            )
            Step.SIGN_IN -> {
                val choice = com.example.croft.mail.onboarding.OAuthProviderChoice.of(state.discovered?.provider)
                val oauthBackend = environment.oauth
                val context = androidx.compose.ui.platform.LocalContext.current
                val oauthScope = androidx.compose.runtime.rememberCoroutineScope()
                // One sign-in controller while S04 shows; leaving S04 ends a pending browser sign-in quietly.
                val oauth = remember(oauthBackend, choice) {
                    if (oauthBackend == null || choice == null) null
                    else com.example.croft.mail.onboarding.OAuthSignIn(
                        oauthBackend, oauthScope,
                        openTab = { url -> openInBrowserTab(context, url) },
                        onSignedIn = { secret -> flow.signInWithOAuth(secret) },
                        onReturned = { returnToCroft(context) },
                    )
                }
                androidx.compose.runtime.DisposableEffect(oauth) { onDispose { oauth?.cancel() } }
                SignInScreen(
                    state = state, onSignIn = flow::signIn, onServerSettings = flow::openServerSettings, onTrust = flow::trustServer,
                    onCancelTrust = flow::cancelTrust, onPasswordEdited = flow::dismissFailure, onBack = { flow.back() },
                    ownClient = if (oauth != null && choice != null && oauthBackend != null) ({
                        val phase by oauth.phase.collectAsState()
                        var remembered by remember(choice) { mutableStateOf<String?>(null) }
                        LaunchedEffect(choice) { remembered = oauthBackend.rememberedClientId(choice) }
                        val donePage = androidx.compose.ui.res.stringResource(com.example.croft.R.string.onb_own_client_done_page)
                        OwnClientSection(
                            provider = choice, phase = phase, rememberedClientId = remembered,
                            onStart = { id, secret -> oauth.start(choice, id, secret, state.email, donePage) },
                            onCancel = oauth::cancel, onEdited = oauth::clearFailure,
                            enabled = state.attempt !is com.example.croft.mail.onboarding.Attempt.Working,
                        )
                    }) else null,
                )
            }
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
                        if (now?.done == true) { fire(account); return@LaunchedEffect }
                        delay(FirstSyncPollMs)
                    }
                }
                FirstSyncScreen(accountName = EmailInput.localPartOf(state.email), status = status, onContinue = { finish() })
            }
        }
    }
}

# Store readiness: Google Play and the App Store

As of 5 November 2026, with rows marked "checked 11 November 2026" updated from `AUDIT-2026-11-11.md`. This file mirrors the Store readiness tab of the plan document. It lists what each store requires of Croft, what changed in the plan because of it, and what to do before each release. Facts come from the stores' own pages, linked under Sources. Anything I could not confirm is marked **Confirm**.

**Short version.** The plan changed in 15 places, all small (next section). Four things need an account, money or lead time, so start them early:

- A Google Play developer account and an Apple Developer Program membership, both paid.
- Google Play's closed test: for a personal account created after 13 November 2023, at least 12 testers opted in for 14 days before production access.
- Apple's default-mail-app entitlement, requested by the Account Holder on a form. Apple decides.
- Google's OAuth verification and security assessment for one shared Gmail sign-in. Until it is done, Google caps the app at 100 new users, so both listings say Gmail needs an app password or your own client. Third-party sources put the assessment at tens of thousands of dollars a year; get a quote before counting on it.

## What changed in the plan

| # | Area | Change | Why |
| --- | --- | --- | --- |
| 1 | Android target | Target Android 16 (API 36) from the first build. Edge-to-edge cannot be opted out of, and predictive back is on by default with the old back-key callback no longer called. | Play requires API 36 for new apps and updates from 31 August 2026. The Back rules in `docs/ux/navigation.md` now say all Back handling uses the system back-callback API. Existing apps must target API 35 or higher to stay available to new users on newer Android versions. An API 37 requirement for 2027 is not yet published; check before August 2027. For API 37 the temporary large-screen opt-out disappears. (checked 11 November 2026) |
| 2 | Large screens | No orientation, resizability or aspect-ratio locks. Layouts work at any window size from 600 dp up. | At API 36 those restrictions are ignored on screens 600 dp wide or more. Build step 12 now says so. |
| 3 | 16 KB pages | Build the Rust core's native libraries for 16 KB pages: NDK r28 or newer with AGP 8.5.1 or newer, or the max-page-size linker flags. CI checks alignment (`android/tools/check-16kb-alignment.sh`) and runs a 16 KB emulator. (checked 11 November 2026) | Apps targeting API 35 or higher must support 16 KB pages on 64-bit devices. From 1 February 2027, updates that do not cannot be released. |
| 4 | Background connection | The Play build has no always-on foreground service. Periodic sync is the default, and a Companion or UnifiedPush gives instant delivery. The F-Droid and GitHub builds may offer an opt-in persistent connection. | This is my plan decision, not a quoted rule: Android 14 and later need a declared foreground-service type, and none is a clean fit for holding IDLE open. **Confirm** with a Play pre-launch review. |
| 5 | Alarms | Snooze uses an inexact alarm plus WorkManager and a boot receiver. The app requests no exact-alarm permission. | Play restricts `USE_EXACT_ALARM` to apps whose core function needs it, such as alarm, timer and calendar-event apps. `SCHEDULE_EXACT_ALARM` needs the person's grant. Snooze does not need second-level precision. |
| 6 | Contacts | No `READ_CONTACTS` by default. Autocomplete uses CardDAV contacts and sent-mail history. "Pick from device contacts" uses the system Contact Picker. | Play's Contacts Permission policy is listed for 27 January 2027. Apps targeting API 37 or higher need a Play Console declaration to keep `READ_CONTACTS`. A separate restriction on the Manage Contacts permission is listed for 25 August 2027. (checked 11 November 2026) |
| 7 | Account deletion | A Companion registration is an account, so S58 gets Delete my Companion data and Unregister this device, and the project publishes a deletion web page. Local use creates no Croft account. | Play requires an in-app path and a web link for apps that let people create accounts, and Apple 5.1.1(v) requires in-app deletion. Accounts created and used only offline are outside Play's policy. |
| 8 | Privacy policy | One policy, linked from S60, from the Play listing and from App Store Connect. A draft is in the repository under docs/store. | Required to complete Play's Data safety form, and by Apple 5.1.1(i). |
| 9 | Review access | S01 has a Try with sample mail button (a built-in demo mode). Review notes also give a real test mailbox on a project-run test server. | Apple 2.1(a) asks for demo account details, and allows a built-in demo mode in place of an account only with prior approval. |
| 10 | Gmail wording | Onboarding and both listings say Gmail needs an app password or your own OAuth client until the shared client is verified. | An unverified app is limited to 100 new users. |
| 11 | iOS push | Push payloads stay content-free. A Notification Service Extension fetches the message and builds the notification text on the phone. Nothing in the app depends on push. | Apple 4.5.4: push must not be required for the app to function and should not carry sensitive or confidential information. |
| 12 | iOS default mail | Request the default-mail-client entitlement. The app registers the mailto: scheme, sends to any valid recipient and receives from any sender. | Apple's entitlement page sets exactly those conditions, and says user-controlled incoming screening is allowed. |
| 13 | iOS build rules | Xcode 26 and the iOS 26 SDK. A privacy manifest that declares the required-reason APIs the app and its libraries use. | Xcode 26 has been required since 28 April 2026. Required-reason declarations have been mandatory since 1 May 2024, including for third-party code. |
| 14 | Sideloaded builds | Register the Android package names and signing keys with Android developer verification, for the GitHub and F-Droid builds as well. | Verification applies on certified devices regardless of where the app came from. Android developer verification applies from 30 September 2026 in Brazil, Indonesia, Singapore and Thailand, with global expansion in 2027. Apps distributed only outside Play register in the Android Developer Console. Apps on and outside Play register in Play Console. Hobbyists can use a limited-distribution account (20 devices). Google's page links an open-source registry guide but does not name F-Droid; check the participating-stores list (https://developer.android.com/developer-verification). (checked 11 November 2026) |
| 15 | Licences | No AGPL code inside the apps. The AGPL Companion and gateway stay server-side. CI runs a licence check on every dependency, and S60 lists open-source licences. | The apps ship through the stores under MPL-2.0. **Confirm** the legal read with a licence review before the first submission. |

## Google Play checklist

| Item | What to do | Notes |
| --- | --- | --- |
| Developer account | Create it and finish identity verification. | Start early. |
| Closed test | Run a closed test with at least 12 testers opted in for at least 14 days, then apply for production. | Stated for personal accounts created after 13 November 2023. Secondary sources say organisation accounts are exempt and that reviewers now look at tester engagement; no official reduction found. **Confirm** in Play Console. (checked 11 November 2026) |
| Target API | targetSdk 36. | The extension to 1 November 2026 has passed. |
| 16 KB pages | Rebuild native libraries; run `android/tools/check-16kb-alignment.sh` on the release APK and adb shell getconf `PAGE_SIZE` on a 16 KB emulator (it prints 16384). Also run Google's RELRO check (GNU_RELRO end must be 16 KB aligned when it shares a writable segment). | Native code means the Rust core's .so files and any prebuilt library. The RELRO check matters only for libraries built with NDK r27 or older, such as prebuilt dependencies. (checked 11 November 2026) |
| Data safety form | Complete it and keep it accurate. Answers are drafted below. | All published apps must. You alone are responsible for its accuracy. |
| Privacy policy | Link it in the listing. | Required to complete the Data safety form. |
| Data deletion | Answer the deletion questions and give the deletion web link. | The link must work and name the app as it appears on Play. |
| App content forms | App access (give test credentials or the sample mailbox), content rating, target audience, ads declaration. | These forms exist in Play Console. I did not read their pages this pass. The Play policy page mentions "October 2026 policy updates" with no detail: read them before submitting. **Confirm**. |
| Permissions | Ship only what the permissions table lists. | No contacts, no exact alarms. |
| Signing | Use Play App Signing with a separate upload key, and register the package and key for developer verification. | The GitHub build uses its own key, also registered. |
| Pre-launch report | Run it on every release candidate. | Part of build step 13. |
| Listing | Real screenshots, no other company's logo, a plain statement of the Gmail limit. | See Names and trademarks. |

## App Store checklist

| Item | What to do | Notes |
| --- | --- | --- |
| Membership | Enrol in the Apple Developer Program. The Account Holder role is needed for entitlements. |  |
| Toolchain | Xcode 26 or later with the iOS 26 SDK. Deployment target iOS 13 or later is the stated floor. | Apple lists both as current requirements. Xcode 26 since 28 April 2026 and iOS 13 minimum from 9 September 2026 are listed on Apple's Upcoming Requirements page (read 11 November 2026; no 2027 items listed). |
| Age rating | Answer the updated age-rating questions when creating the app. | The deadline for existing apps was 31 January 2026. |
| EU trader status | Declare whether you are a trader before shipping in the EU. | Apps without verified trader status were removed from the EU store from 17 February 2025. **Confirm** what is shown publicly for a trader. |
| Privacy policy | Link it in App Store Connect and in the app (S60). | It must say what data is collected, how, and all uses; list third parties and their protections; and explain retention, deletion and revoking consent. |
| App Privacy answers | Drafted below. | Tracking: none. |
| Privacy manifest | Add PrivacyInfo.xcprivacy to the app and to any framework you ship. | Template in docs/store. |
| Account deletion | S58, in the app. | 5.1.1(v). |
| Review notes | Demo mailbox details, the sample-mail mode, and a sentence on the default-mail entitlement. | 2.1(a). |
| Sign in with Apple | Not required. | 4.8 exempts "a client for a specific third-party service where users are required to sign in to their mail ... account directly", and apps that use only their own account system. If the Companion ever offers "Sign in with Google" for its own account, 4.8 applies. (checked 11 November 2026) |
| Push | Optional, content-free, no marketing. | 4.5.4. |
| Entitlement | Request the default-mail-client entitlement. | Account Holder form. Apple decides. |
| Export compliance | Answer the encryption questions in App Store Connect. | **Confirm**: I could not read Apple's export page. See below. |
| TestFlight | Beta before submission. |  |
| Accessibility | Apple's Accessibility Nutrition Labels are voluntary for now, and the plan already supports VoiceOver, Dynamic Type and Reduce Motion. | **Confirm** the current status before each release. |

## Data safety and App Privacy answers (draft)

These are conservative drafts from the plan as written. The stores hold the developer alone responsible for the answers, so check each against the finished build before submitting.

| Data | Where it goes | Draft answer |
| --- | --- | --- |
| Emails, attachments, calendar events, contacts | Between the phone and the servers the person adds, and to their own Companion if they run one. Never to the project. | Not collected by the developer; the privacy policy says so in plain words. If a reviewer reads "sent to the person's own server" as collection, the fallback is to declare Emails, Calendar events and Contacts as collected for App functionality, encrypted in transit, with deletion available. |
| Push token and an opaque account ID | To the push gateway, so it can wake the phone. FCM and APNs process the token. | Collected: Device or other IDs. Purpose: App functionality. Not used for advertising or tracking. Optional, because periodic sync works without it. On the App Store, declare Device ID as linked to the person. |
| Crash logs, diagnostics, analytics | None. There are no analytics or advertising libraries. | Not collected. Opt-in crash reporting added later changes this answer. |
| Location, photos, files, financial, health, web history | None. | Not collected. |
| Security practices | TLS is required for mail connections. Deletion is in S58 and on the web page. | Encrypted in transit: yes. Deletion request: yes. |

## Permissions and usage strings

| Platform | Permission | When and why |
| --- | --- | --- |
| Android | INTERNET | Mail, calendar and contacts sync. Declared since T16 (11 November 2026). |
| Android | `ACCESS_NETWORK_STATE` | Knowing when the device is offline, so actions wait instead of failing (T16). Normal permission, no prompt. |
| Android | `POST_NOTIFICATIONS` | Asked after the first account is added and the person opts into notifications. |
| Android | `USE_BIOMETRIC` | App lock. |
| Android | `RECEIVE_BOOT_COMPLETED` | Restores snooze alarms after a restart. |
| Android | CAMERA | Only when scanning a QR code for settings or key import. Asked at that moment. |
| Android | Not requested | `READ_CONTACTS`, exact alarms, foreground-service permissions in the Play build, storage access. |
| iOS | Face ID usage string | App lock. |
| iOS | Camera usage string | QR scanning. |
| iOS | Notifications | Asked after the first account is added. |
| iOS | Contacts usage string | Only if "Pick from device contacts" is added; the system picker needs none. |

## Licences and export

**Licences.** Apps and core are MPL-2.0. The Companion and gateway are AGPL-3.0 and are never shipped inside the apps. CI fails the build if a dependency with an incompatible licence reaches an app target, and the app's licence screen lists every dependency.

**Export.** The app uses encryption beyond the operating system's own (OpenPGP and S/MIME in the Rust core), so App Store Connect's export-compliance questions are unlikely to be a plain "exempt". **Confirm**: I could not read Apple's export-compliance page, and what follows is background knowledge. Expect to answer that the app uses non-exempt encryption, and to file the annual self-classification report if the rules require it. Play has a similar export-laws declaration. Apple's page returned no usable text on 11 November 2026, so this is still unchecked. Background knowledge only: apps with their own cryptography generally answer "non-exempt", and may need an annual US self-classification report and French documentation. Get this checked before the first submission.

## Default mail app on iOS

The entitlement is com.apple.developer.mail-client. Sign in as the Account Holder and use Apple's request form. The app must register the mailto: scheme, be able to send to any valid recipient, and be able to receive from any sender. User-controlled screening of incoming mail is allowed, which covers Croft's block-sender feature. Test the entitlement in a TestFlight build early; Apple's developer forums show people hitting build-signing problems with it.

## Review access

Apple asks for demo account details and a running back end (2.1(a)). It allows a built-in demo mode in place of an account only with prior approval, so Croft does both. The project runs a small test mail server (the same containers as the integration tests) with a reviewer mailbox and fills the review notes with its address and password. S01's Try with sample mail opens a local, offline sample mailbox with fictional people, so a reviewer who cannot sign in still reaches every screen. Use the same mailbox for Play's App access declaration.

## Account deletion

Croft itself has no accounts unless a person registers with a Companion. Mail accounts belong to their provider, and removing one in Croft deletes its local data. For a Companion, S58 offers Unregister this device (this phone only) and Delete my Companion data (the account and everything stored for it), each confirmed first. A web page at the project's address explains how to request the same deletion without the app, and the Play listing and privacy policy link to it.

## Names, logos and trademarks

Apple's 4.1(c) bars using another developer's icon, brand or product name in an app's icon or name without approval. So the icon and name use only Croft's own marks, and "Gmail", "Outlook" and "iCloud" appear only as plain words naming what the app connects to, never as logos. Screenshots use fictional people and domains (2.3.9). I have not searched trademark registers for "Croft" in software and mail classes. Do that before paying for store accounts and listings.

## Open items

- Confirm the export-compliance answers for Apple and Play.
- Confirm whether a client that keeps Gmail data only on the device needs Google's security assessment. Google's page ties the requirement to restricted data stored or transmitted through a server and does not address the device-only case. The Companion, if it proxies Gmail, is a server.
- Confirm that no foreground service type is needed for any Play-build feature, with a pre-launch review.
- Read Play's App access, content rating and target audience pages and complete those forms.
- Android developer verification: applies from 30 September 2026 in Brazil, Indonesia, Singapore and Thailand, with global expansion in 2027. Apps distributed only outside Play register in the Android Developer Console; apps on and outside Play register in Play Console; hobbyists can use a limited-distribution account (20 devices). Google's page does not name F-Droid: check the participating-stores list (checked 11 November 2026).
- Data safety and App content forms: **Confirm**. Read the October 2026 policy updates before submitting.
- Trademark search for "Croft".

## Sources

- [Google Play target API requirements](https://developer.android.com/google/play/requirements/target-sdk)
- [16 KB page size support](https://developer.android.com/guide/practices/page-sizes)
- [Android 16 behavior changes](https://developer.android.com/about/versions/16/behavior-changes-16)
- [Android developer verification](https://developer.android.com/developer-verification/guides)
- [Play closed testing for new personal accounts](https://support.google.com/googleplay/android-developer/answer/14151465)
- [Play account deletion requirements](https://support.google.com/googleplay/android-developer/answer/13327111)
- [Play Data safety form](https://support.google.com/googleplay/android-developer/answer/10787469)
- [Play permissions and sensitive APIs policy](https://support.google.com/googleplay/android-developer/answer/9888170)
- [Google restricted scope verification](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification)
- [Google unverified app limits](https://support.google.com/cloud/answer/7454865)
- [Apple upcoming requirements](https://developer.apple.com/news/upcoming-requirements/)
- [App Review Guidelines](https://developer.apple.com/app-store/review/guidelines/)
- [Default mail client entitlement](https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.mail-client)

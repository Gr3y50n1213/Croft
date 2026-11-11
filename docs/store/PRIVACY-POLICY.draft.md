# Croft privacy policy (DRAFT)

> **Draft for the project owner and a lawyer to review.** Replace the bracketed placeholders, host the
> final text at a stable public address, and link it from S01, S60, the Google Play listing and App Store
> Connect. The wording describes the product as planned in `docs/PLAN.md`; check every sentence against
> the finished build before publishing, because the stores hold the developer responsible for accuracy.

Effective: [date]. Publisher: [legal name and contact address].

## The short version

- Croft has no Croft account. You sign in to your own email provider or mail server.
- Your mail goes between your phone or browser and the servers you add. It does not pass through the
  Croft project's servers.
- Croft contains no analytics, no advertising and no tracking.
- You can delete everything Croft stores, on the device and on a Companion, from inside the app.

## What stays on your device

Croft stores your messages, folders, contacts, calendar items, settings and drafts in an encrypted-at-rest
database on the device. Passwords, OAuth tokens and private keys are kept only in the operating system's
secure keystore (Android Keystore, iOS Keychain, or the browser's secure storage on the web).
Removing an account in Croft deletes that account's local data. Uninstalling the app deletes the rest.

## What goes to the servers you choose

When you add an account, Croft connects to your mail provider or server (over TLS only) to read,
send and change your mail, as you instruct. That traffic is between you and them. Their privacy policy
applies to what they do with it. For Gmail and Outlook accounts, Croft uses the provider's own sign-in
and asks only for the access needed to read and send mail, and the data stays on your device.
[Confirm the exact Google API scopes and their stated use before publishing.]

## Companion server (optional)

If you connect a Croft Companion, run by you or by someone you trust, it holds a copy of the mail it
needs to give you search, push and web access, and the account details you register. Whoever runs the
Companion controls that data. Croft the project does not receive it. The Companion's own retention and
deletion are in S58: **Unregister this device** removes this phone, and **Delete my Companion data**
removes your account and everything stored for it.

## Push notifications (optional)

Push is optional. Periodic sync works without it. With a Companion, a notification ping can reach your
phone in one of two ways:

- **UnifiedPush:** through the distributor you choose. The project's gateway is not involved.
- **Google or Apple push (FCM, APNs):** through the project's push gateway. The gateway receives a
  device push token, an opaque account identifier and the time of each ping. It never receives message
  content, sender, subject or addresses. The ping is encrypted and says only that there is something to
  check, and your phone then fetches the message itself. Google and Apple process the token to deliver
  the ping. [State the gateway's log retention, for example "no ping logs; error logs for 7 days".]

## What Croft does not collect

No analytics, no advertising identifiers, no crash reports sent anywhere, no location, no photos or
files unless you attach them to a message, no payment data. If an opt-in crash report is ever added, this
policy will change before it ships, and it will be off until you turn it on.

Diagnostics you copy from S60 are redacted on the device (no passwords, tokens, message text or full
addresses) and go nowhere unless you share them yourself.

## Remote content

Images and other remote content in a message are blocked until you allow them, so senders cannot learn
that you opened a message or where you are.

## Permissions

- Notifications: to tell you about new mail, if you choose.
- Biometrics (Face ID, fingerprint): to unlock the app, if you choose.
- Camera: only when you scan a QR code for settings or a key, asked at that moment.
- Contacts: not requested. Croft completes addresses from your mail history and from CardDAV contacts you add.
  If you choose "Pick from device contacts", the system contact picker shares only the contact you select.

## Your choices and deletion

- Remove an account or reset the app in Croft at any time (S60).
- Unregister this device or delete your Companion data in S58.
- To request deletion without the app, follow [deletion page address], which explains the steps and
  the contact address. We act on requests within [30] days.
- Mail stored by your provider or your own server is deleted there, with their tools.

## Children

Croft is a general-purpose email client and is not directed at children under 13 (or the higher age your
country sets).

## Changes and contact

We will describe changes here and, for material ones, in the app. Questions: [contact address].
Croft is not affiliated with Google, Microsoft or Apple.

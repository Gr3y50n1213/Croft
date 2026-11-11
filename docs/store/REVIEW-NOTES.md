# Review notes (Apple App Review and Google Play App access)

Paste the text below into App Store Connect's "Notes" and "Sign-in required" fields and into Play Console's
App access. Fill the bracketed values from the test server before each submission, and keep the test
server running for the whole review. Rules: Apple 2.1(a) (demo account or approved demo mode), Play App access.

## Text to paste

Croft is an email client. It connects to the person's own email account or mail server, so it has no
sign-in of its own.

**Quickest way to see every screen.** On the first screen tap **Try with sample mail**. This opens a
read-only mailbox made on the device from fictional people and domains. It needs no account and no network.

**Real account for testing.** Add this mailbox from **Add your email**:

- Address: [reviewer address]
- Password: [reviewer password]
- Server: found automatically from the address ([test server host]).

The mailbox holds a few messages, a draft, an attachment and a calendar invitation, and is reset
every day.

**Push notifications** are optional and not needed to review the app. Periodic sync works without them.
Push carries no message content.

**Default mail app (iOS).** The app registers the mailto: scheme, can send to any valid recipient and
receives from any sender, and we have requested the default mail client entitlement. User-controlled
sender blocking is the only filtering of incoming mail.

**Account deletion.** Settings, Companion and devices (S58), then **Delete my Companion data**. A Croft
account exists only if the person registers with a Companion. Removing a mail account (S43) deletes its local data.

**Encryption.** The app uses OpenPGP and S/MIME for optional message encryption, in addition to TLS.
[Insert the export-compliance answers confirmed in READINESS.md.]

**Contact:** [name, phone, email]

## Before every submission

1. Start the test mail server (`docker compose -f test-servers/docker-compose.yml up -d`, or the hosted copy)
   and check the reviewer mailbox signs in from a clean install.
2. Put the sample-mail mode through a clean install on a phone with no network.
3. Confirm the privacy policy and deletion pages load and name the app exactly as the listing does.
4. Update the data answers if anything collects data that the last release did not.

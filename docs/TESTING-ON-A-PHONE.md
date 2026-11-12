# Testing Croft on your Android phone

This guide is for the owner. You do not need to know Git or GitHub. Take your time. Each step is small.

## What this build is

- It is **Croft Preview**, a test build. It is not the final app.
- It is signed with a **test key** (the standard Android debug key), not the key of the future store version. So it **cannot be updated into the Play Store version**. The store version will install as a separate app next to it.
- It installs next to other Croft builds. The app is called **Croft Preview** on your home screen.
- It sends nothing anywhere except to **your own mail server**. There are no analytics and no tracking.
- Each new preview build is signed with a different test key. To install a newer preview, **remove the old Croft Preview first** (see "How to remove it"). Removing it deletes the app's data from the phone. Your mail on the server is not touched.

## How to download it

1. On your phone or computer, open a browser and go to the project's GitHub page. Sign in to GitHub if it asks.
2. Tap **Actions** (in the row of tabs near the top).
3. Find the latest run of **ci** that has a green tick. Use a run on `main`, or use the link you were given. Tap its name.
4. Scroll to the bottom of the page, to **Artifacts**.
5. Tap **croft-preview-apk**. A zip file downloads.
6. Unzip it. On the phone: open the **Files** app, tap the zip, then tap **Extract**. On a computer: double-click the zip, then copy the file out to the phone (by cable, or send it to yourself).
7. Inside is one file, `app-preview.apk`. This is the app.

Artifacts are kept for 30 days. If the page says the artifact has expired, ask for a new run.

## How to install it

1. Tap `app-preview.apk` in the Files app (or in the browser's downloads).
2. Android will say it cannot install apps from this source. Tap **Settings** in that message.
3. Turn on **Allow from this source** for the app you used (for example Files or Chrome). If you do this by hand: **Settings → Apps → Special app access → Install unknown apps**, then choose that app. The exact words differ a little between phone makes.
4. Go back and tap **Install**.
5. If **Play Protect** warns about an unknown app, that is expected for a test build. Tap **More details**, then **Install anyway**.
6. When it has finished, tap **Open**.
7. **Afterwards, turn "Allow from this source" off again** in the same place. This is a good habit. Croft keeps working.

## First steps

1. Open **Croft Preview**.
2. To use your own mail, tap **Add an account**. Type your email address and tap **Continue**. Croft looks up the server settings.
3. Sign in with your username and password.
   - Best for this test: a mail provider that offers **app passwords**, or a server you run yourself.
   - **Gmail and Outlook** do not accept your normal password in apps like this. Create an **app password** in your Google or Microsoft account and paste it in. A "Sign in with Google/Microsoft" button comes in a later version.
   - If Croft asks **Trust this server?**, only tap **Trust this server** if you set the server up yourself and the fingerprint matches.
4. Prefer not to use a real account yet? Tap **Try with sample mail**. It shows made-up messages. Nothing is sent or saved to a server.

## What to test

Tick these off as you go.

- [ ] Your mail appears in the Inbox. Scroll through it.
- [ ] Open a message and read it.
- [ ] A message with pictures shows "Remote images blocked". Pictures stay hidden until you tap **Show images**.
- [ ] Reply to a message.
- [ ] Add an attachment to a message you write.
- [ ] Send a message to yourself. It arrives.
- [ ] Tap **Undo** right after sending, before the time runs out. The message is not sent.
- [ ] Open **Outbox**. A message waiting to go out is listed there.
- [ ] Archive or delete a message, then tap **Undo**.
- [ ] Add a second account, then remove it: **Settings → Accounts → Remove**.
- [ ] Last: **About and diagnostics → Erase all Croft data on this device**. Everything on the phone is deleted and Croft starts fresh.

## Known gaps

These are not done yet. You do not need to report them.

- There is no search.
- There are no notifications and no background sync. **Open the app to fetch new mail.**
- There is no Gmail or Outlook sign-in button.
- There is no scheduled send.

## How to report a problem

Send these three things:

1. **What happened**, and what you expected.
2. **The steps** to make it happen again.
3. **A screenshot.** Hide anything private first.

**Never send a password, and never send the content of private emails.**

To help us, open **About and diagnostics** and tap **Copy diagnostics**. Paste the text into your report. It contains no personal data (no passwords, no addresses, no message text).

## How to remove it

1. If you want everything gone, first open **About and diagnostics** and tap **Erase all Croft data on this device**, then **Erase everything**. Your mail on the servers is not touched.
2. Press and hold **Croft Preview** on your home screen. Tap **App info**, then **Uninstall**.
3. If you turned "Install unknown apps" on for the browser or Files app, turn it off again now.

# Deleting your data (draft)

The page the store listings link to for deletion (Google Play's "Delete account URL"). Host it beside the privacy policy and put its
address in the privacy policy's "[deletion page address]" (OPEN-ITEMS).

---

## Deleting your data in Croft

Croft has no central account. Croft the project runs no server and receives none of your data.

**On your phone.** Your mail, accounts and settings stay on your phone.
- To remove one mail account, open Settings, Accounts, choose the account, then Remove.
- To remove everything, open Settings, About, then "Erase all Croft data", or uninstall the app.

**On a Croft Companion.** If you connected a Companion, it holds an account for you: a copy of your mail, your mail accounts and
their passwords, your passkeys and your signed-in devices. To delete it:
1. In the app, open Settings, Companion, then "Delete my Companion data". This opens your Companion in the browser. Or open
   `https://<your Companion's address>/delete` in any browser.
2. Sign in to your Companion. For your safety it asks for a sign-in from the last ten minutes.
3. Type your user name and confirm. Everything is deleted at once and cannot be restored. Your mail on your mail servers is not
   touched.

**If you can no longer sign in** to your Companion, ask the person who runs it. They can delete your account with
`croft-companion user delete --name <your user name>` (`companion/README.md`).

Mail stored by your email provider or your own mail server is deleted there, with its own tools.

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Sending mail, as the engine sees it: [`MailSubmitter`].
//!
//! It is separate from [`MailBackend`]: reading and changing a mailbox is one
//! job, handing a message to a server for delivery another, and some protocols do both on one
//! connection (JMAP `EmailSubmission/set`) while others use two servers (SMTP to send, IMAP to
//! keep the copy in Sent). The engine only knows the three calls below.
//!
//! [`SmtpImapSubmitter`] is the SMTP and IMAP implementation: [`SmtpSender`] submits, and the copy
//! for the Sent folder is appended over the account's [`ImapBackend`] unless the server files it
//! itself (Gmail does).

use std::sync::Arc;

use crate::{
    BackendError, BackendFuture, ImapBackend, MailBackend, RemoteFolder, SmtpSender, SpecialUse,
};

/// Name of the folder created when the account has none marked (or named) as Sent.
pub const DEFAULT_SENT_FOLDER: &str = "Sent";

/// Reason given when the copy for Sent cannot be appended because the message is not valid UTF-8
/// (the IMAP adapter's `append_message` takes text). A documented limit: the composer should write
/// 7-bit or UTF-8 MIME.
pub const REASON_COPY_NOT_TEXT: &str =
    "The copy for the Sent folder needs a message in UTF-8 or 7-bit text";

/// Hands one message to a server for delivery and keeps a copy in Sent.
///
/// Errors follow the rest of the protocol layer: [`BackendError::Transient`] is retried,
/// [`BackendError::Auth`] pauses the account until the person signs in again,
/// [`BackendError::Permanent`] fails the message. They never contain addresses or message text.
///
/// An implementation must bound [`MailSubmitter::send`] by its own deadline: the engine never
/// cancels a send that is in flight (a cancelled send may have reached the server).
pub trait MailSubmitter: Send + Sync {
    /// Submits one message. `envelope_from` and `recipients` are bare addresses (Bcc recipients
    /// included, and not in the headers); `mime` is the whole RFC 5322 message, exactly as stored.
    ///
    /// A failure after the server may already have taken the message must be
    /// [`BackendError::Permanent`] with the "may not have been sent" reason, never `Transient`: the
    /// engine retries `Transient` on its own, and that would send the message twice.
    fn send<'a>(
        &'a self,
        envelope_from: &'a str,
        recipients: &'a [String],
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()>;

    /// True when the server saves a copy in Sent by itself after [`MailSubmitter::send`], so
    /// [`MailSubmitter::file_in_sent`] must not be called.
    fn files_sent_itself(&self) -> bool;

    /// Saves a copy of the sent message in the Sent folder, **once**: when `message_id` is given
    /// the folder is searched for it first, and nothing is added if it is there (a crash in the
    /// middle of an earlier attempt may already have saved it). Creates the folder if the account
    /// has no Sent folder.
    fn file_in_sent<'a>(
        &'a self,
        message_id: Option<&'a str>,
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()>;
}

/// Sends over SMTP and files the copy over IMAP.
pub struct SmtpImapSubmitter {
    /// Submits the message.
    pub smtp: SmtpSender,
    /// The account's IMAP adapter, used to look in Sent and to append the copy.
    pub imap: Arc<ImapBackend>,
    /// True for a server that saves the copy itself (Gmail): nothing is appended.
    pub files_sent_itself: bool,
}

impl std::fmt::Debug for SmtpImapSubmitter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpImapSubmitter")
            .field("smtp", &self.smtp)
            .field("imap", &self.imap)
            .field("files_sent_itself", &self.files_sent_itself)
            .finish()
    }
}

impl SmtpImapSubmitter {
    /// Puts the three parts together.
    pub fn new(smtp: SmtpSender, imap: Arc<ImapBackend>, files_sent_itself: bool) -> Self {
        Self {
            smtp,
            imap,
            files_sent_itself,
        }
    }

    /// The path of the Sent folder, created if there is none.
    async fn sent_folder(&self) -> Result<String, BackendError> {
        let folders = self.imap.list_folders().await?;
        if let Some(found) = pick_sent(&folders) {
            return Ok(found.path.clone());
        }
        self.imap.create_folder(DEFAULT_SENT_FOLDER).await?;
        Ok(DEFAULT_SENT_FOLDER.to_owned())
    }
}

/// The folder to file sent mail in: the one the server marks `\Sent` (or, failing that, one whose
/// name says so), and selectable.
fn pick_sent(folders: &[RemoteFolder]) -> Option<&RemoteFolder> {
    folders
        .iter()
        .find(|f| f.special_use == SpecialUse::Sent && f.selectable)
}

impl MailSubmitter for SmtpImapSubmitter {
    fn send<'a>(
        &'a self,
        envelope_from: &'a str,
        recipients: &'a [String],
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            self.smtp
                .send(envelope_from, recipients, mime)
                .await
                .map(|_| ())
        })
    }

    fn files_sent_itself(&self) -> bool {
        self.files_sent_itself
    }

    fn file_in_sent<'a>(
        &'a self,
        message_id: Option<&'a str>,
        mime: &'a [u8],
    ) -> BackendFuture<'a, ()> {
        Box::pin(async move {
            let text = std::str::from_utf8(mime)
                .map_err(|_| BackendError::Permanent(REASON_COPY_NOT_TEXT.to_owned()))?;
            let folder = self.sent_folder().await?;
            if let Some(id) = message_id {
                if self.imap.find_by_message_id(&folder, id).await?.is_some() {
                    return Ok(());
                }
            }
            self.imap
                .append_message(&folder, &["\\Seen"], text)
                .await
                .map(|_| ())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(path: &str, special_use: SpecialUse, selectable: bool) -> RemoteFolder {
        RemoteFolder {
            path: path.into(),
            delimiter: Some('/'),
            special_use,
            selectable,
        }
    }

    #[test]
    fn the_sent_folder_is_the_selectable_one_marked_sent() {
        let all = [
            folder("INBOX", SpecialUse::Inbox, true),
            folder("Gesendet", SpecialUse::Sent, true),
            folder("Sent", SpecialUse::Other, true),
        ];
        assert_eq!(pick_sent(&all).map(|f| f.path.as_str()), Some("Gesendet"));
    }

    #[test]
    fn a_container_that_cannot_be_selected_is_not_the_sent_folder() {
        let all = [folder("Sent", SpecialUse::Sent, false)];
        assert!(pick_sent(&all).is_none());
        assert!(pick_sent(&[]).is_none());
    }
}

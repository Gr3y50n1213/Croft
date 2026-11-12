// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A draft's copy in the server's Drafts folder.
//!
//! A draft keeps one Message-ID for its whole life, and that is how its copy on the server is found
//! (among the messages flagged `\Draft` only, so a message that merely has the same id is never
//! touched):
//! no stored UID is trusted (a UID means nothing after the folder's UIDVALIDITY changed, and a
//! wrong one would delete somebody else's message). Saving is **look up, append, delete the old
//! one**, in that order, so the server never has less than one copy of the draft; a crash between
//! the append and the delete leaves two copies, and the next save or the final clean-up removes the
//! older one.
//!
//! All three calls are safe to repeat. None of them puts a message, an address or a header in an
//! error.

use crate::{BackendError, ImapBackend, MailBackend, SpecialUse, Uid};

/// The name of the folder created when the account has none marked (or named) as Drafts.
pub const DEFAULT_DRAFTS_FOLDER: &str = "Drafts";

/// Reason given when a draft is not valid UTF-8 (the IMAP adapter's `append_message` takes text).
pub const REASON_DRAFT_NOT_TEXT: &str = "The draft copy needs a message in UTF-8 or 7-bit text";

/// How many copies one clean-up deletes at most. A draft has one copy, or two after a crash; the
/// limit only stops a server that never lets a message go.
const MAX_COPIES_DELETED: usize = 8;

/// Where the copy of a draft is after a save.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftCopy {
    /// The folder it is in.
    pub folder: String,
    /// Its UID there, when the server reported it (`UIDPLUS`) or the look-up found it.
    pub uid: Option<Uid>,
}

/// The path of the Drafts folder: `preferred` (the app's folder mapping) when it is given,
/// otherwise the selectable folder the server marks `\Drafts`, otherwise a folder named
/// [`DEFAULT_DRAFTS_FOLDER`], which is created.
pub async fn drafts_folder(
    imap: &ImapBackend,
    preferred: Option<&str>,
) -> Result<String, BackendError> {
    if let Some(path) = preferred.filter(|p| !p.is_empty()) {
        return Ok(path.to_owned());
    }
    let folders = imap.list_folders().await?;
    if let Some(found) = folders
        .iter()
        .find(|f| f.special_use == SpecialUse::Drafts && f.selectable)
    {
        return Ok(found.path.clone());
    }
    imap.create_folder(DEFAULT_DRAFTS_FOLDER).await?;
    Ok(DEFAULT_DRAFTS_FOLDER.to_owned())
}

/// Saves the draft: appends `mime` to `folder` with `\Draft` and `\Seen`, then deletes the copy
/// that was there before (found by `message_id`, the angle brackets included). Returns where the new
/// copy is. `mime` must be 7-bit or UTF-8 text.
pub async fn save_draft_copy(
    imap: &ImapBackend,
    folder: &str,
    message_id: &str,
    mime: &[u8],
) -> Result<DraftCopy, BackendError> {
    let text = std::str::from_utf8(mime)
        .map_err(|_| BackendError::Permanent(REASON_DRAFT_NOT_TEXT.to_owned()))?;
    // The copy to replace, found before the new one is added (afterwards both have the id).
    let old = imap.find_draft_by_message_id(folder, message_id).await?;
    let appended = imap
        .append_message(folder, &["\\Draft", "\\Seen"], text)
        .await?;
    let new = match appended {
        Some(uid) => Some(uid),
        // No `UIDPLUS`: the newest message with the id is the one just added.
        None => imap.find_draft_by_message_id(folder, message_id).await?,
    };
    if let Some(old) = old.filter(|old| Some(*old) != new) {
        imap.delete_forever(folder, &[old]).await?;
    }
    Ok(DraftCopy {
        folder: folder.to_owned(),
        uid: new,
    })
}

/// Deletes every copy of the draft in `folder` (a draft has one, or two after a crash during a
/// save). Returns how many were deleted. Repeating it is harmless.
pub async fn delete_draft_copies(
    imap: &ImapBackend,
    folder: &str,
    message_id: &str,
) -> Result<usize, BackendError> {
    let mut deleted = 0;
    while deleted < MAX_COPIES_DELETED {
        let Some(uid) = imap.find_draft_by_message_id(folder, message_id).await? else {
            break;
        };
        imap.delete_forever(folder, &[uid]).await?;
        deleted += 1;
    }
    Ok(deleted)
}

-- This Source Code Form is subject to the terms of the GNU Affero General Public License,
-- version 3 or later. See LICENSE-AGPL-3.0 at the repository root.
--
-- T25: identities at an OpenID Connect provider linked to Companion users. The identity is the
-- provider's (issuer, subject) pair, never an e-mail address. Forward-only.

CREATE TABLE oidc_identity (
    user_id   INTEGER NOT NULL REFERENCES user(id) ON DELETE CASCADE,
    issuer    TEXT    NOT NULL,
    subject   TEXT    NOT NULL,
    linked_ms INTEGER NOT NULL,
    PRIMARY KEY (issuer, subject)
);
-- One identity per user and provider.
CREATE UNIQUE INDEX oidc_identity_user ON oidc_identity(user_id, issuer);

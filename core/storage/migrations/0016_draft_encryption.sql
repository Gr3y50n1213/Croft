-- T41c: whether a draft is to be sent encrypted and signed (OpenPGP). Decrypted text is never stored anywhere.
ALTER TABLE draft ADD COLUMN encrypt INTEGER NOT NULL DEFAULT 0 CHECK (encrypt IN (0, 1));
ALTER TABLE draft ADD COLUMN sign INTEGER NOT NULL DEFAULT 0 CHECK (sign IN (0, 1));

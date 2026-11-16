# croft-crypto

OpenPGP for Croft (T41a), over [rPGP](https://github.com/rpgp/rpgp): keys (`keys`), PGP/MIME (`mime`), Autocrypt headers
(`autocrypt`) and WKD addresses (`wkd`). Pure: no storage, no network, no clock. Secret key bytes are wiped when dropped and
never appear in `Debug` output or errors. The platform keystore holds secret keys (T41b); this crate never stores anything.

- `tools/make_fixtures.sh` makes the GnuPG test fixtures (made-up people, made-up passphrase).
- `tools/check_with_gpg.sh` checks with GnuPG that it decrypts and verifies what Croft writes.

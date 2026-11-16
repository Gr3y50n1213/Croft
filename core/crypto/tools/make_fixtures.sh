#!/usr/bin/env bash
# Makes the GnuPG interoperability fixtures of croft-crypto's tests (T41a): made-up people, made-up passphrases, throwaway keys.
# Run from core/crypto: bash tools/make_fixtures.sh. Needs GnuPG 2.2 or later. Overwrites tests/fixtures/gpg-*.
set -euo pipefail
out="$(pwd)/tests/fixtures"
home="$(mktemp -d)"
trap 'rm -rf "$home"' EXIT
export GNUPGHOME="$home"
chmod 700 "$home"
pass="made-up passphrase"   # made up, for these test keys only
g() { gpg --batch --yes --pinentry-mode loopback --passphrase "$pass" "$@"; }

# Alice: Ed25519 and Curve25519 (what Croft generates too), protected with the passphrase.
g --quick-gen-key "Alice Example <alice@example.org>" ed25519 cert,sign never
afp=$(gpg --list-keys --with-colons alice@example.org | awk -F: '/^fpr/ {print $10; exit}')
g --quick-add-key "$afp" cv25519 encr never
# Bob: RSA 3072 (older keys are often RSA).
g --quick-gen-key "Bob Example <bob@example.net>" rsa3072 cert,sign never
bfp=$(gpg --list-keys --with-colons bob@example.net | awk -F: '/^fpr/ {print $10; exit}')
g --quick-add-key "$bfp" rsa3072 encr never
# Carol: made in 2020 with a one-day expiry, so expired now.
g --faked-system-time 20200101T000000 --quick-gen-key "Carol Example <carol@example.com>" ed25519 cert,sign 1d
cfp=$(gpg --list-keys --with-colons carol@example.com | awk -F: '/^fpr/ {print $10; exit}')
g --faked-system-time 20200101T000000 --quick-add-key "$cfp" cv25519 encr 1d
# Dave: revoked by his owner.
g --quick-gen-key "Dave Example <dave@example.com>" ed25519 cert,sign never
dfp=$(gpg --list-keys --with-colons dave@example.com | awk -F: '/^fpr/ {print $10; exit}')
g --quick-add-key "$dfp" cv25519 encr never
# GnuPG writes a revocation certificate for every new key; its first line is guarded with a colon.
sed 's/^:-----BEGIN/-----BEGIN/' "$home/openpgp-revocs.d/$dfp.rev" > "$home/dave.rev"
gpg --batch --import "$home/dave.rev"


gpg --armor --export alice@example.org > "$out/gpg-alice.pub.asc"
g --armor --export-secret-keys alice@example.org > "$out/gpg-alice.sec.asc"
gpg --armor --export bob@example.net > "$out/gpg-bob.pub.asc"
g --armor --export-secret-keys bob@example.net > "$out/gpg-bob.sec.asc"
gpg --armor --export carol@example.com > "$out/gpg-carol-expired.pub.asc"
gpg --armor --export dave@example.com > "$out/gpg-dave-revoked.pub.asc"

# Bob signs and encrypts to Alice; Alice signs a text detached (as a multipart/signed part would be).
printf 'Content-Type: text/plain; charset=utf-8\r\n\r\nHello Alice, from Bob.\r\n' > "$home/inner.txt"
g --armor --sign --local-user "$bfp" --encrypt --recipient "$afp" --output "$out/gpg-bob-to-alice.asc" "$home/inner.txt"
cp "$home/inner.txt" "$out/gpg-signed-part.txt"
g --armor --detach-sign --local-user "$afp" --digest-algo SHA256 --output "$out/gpg-signed-part.sig.asc" "$home/inner.txt"
echo "fixtures written to $out"

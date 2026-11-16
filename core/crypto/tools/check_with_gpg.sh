#!/usr/bin/env bash
# Checks with GnuPG what Croft writes (T41a): a message Croft encrypted and signed to the fixture key Alice, and a part Croft
# signed. Run from core/crypto: bash tools/check_with_gpg.sh. Needs GnuPG and cargo.
set -euo pipefail
out="$(mktemp -d)"; home="$(mktemp -d)"; trap 'rm -rf "$out" "$home"' EXIT
export GNUPGHOME="$home"; chmod 700 "$home"
CROFT_INTEROP_OUT="$out" cargo test --offline -p croft-crypto --test openpgp writes_samples_for_gnupg_when_asked -q >/dev/null
gpg --batch --quiet --pinentry-mode loopback --passphrase "made-up passphrase" --import tests/fixtures/gpg-alice.sec.asc
gpg --batch --quiet --import "$out/croft-sam.pub.asc"
echo "== decrypt (expect the text and a good signature from Sam)"
gpg --batch --pinentry-mode loopback --passphrase "made-up passphrase" --decrypt "$out/croft-to-alice.asc"
echo "== verify the signed part (expect a good signature from Sam)"
gpg --batch --verify "$out/croft-signed-part.sig.asc" "$out/croft-signed-part.txt"

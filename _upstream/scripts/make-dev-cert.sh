#!/usr/bin/env bash
# OPT-IN. Creates a self-signed code-signing identity named "notscreenstudio-dev" in your login keychain, so
# scripts/bundle.sh can sign every build with the same identity and macOS keeps Screen Recording, Camera and
# Microphone permission across rebuilds. Run it yourself, once. Nothing else in this repo runs it.
set -euo pipefail

NAME="notscreenstudio-dev"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"

# Capture first so `grep -q` closing the pipe cannot trip pipefail. Same check as scripts/bundle.sh.
VALID="$(security find-identity -v -p codesigning 2>/dev/null || true)"
ALL="$(security find-identity -p codesigning 2>/dev/null || true)"
if printf '%s\n' "$VALID" | grep -q "\"$NAME\""; then
  echo "Identity \"$NAME\" already exists and is valid for code signing. Nothing to do."
  exit 0
fi
if printf '%s\n' "$ALL" | grep -q "\"$NAME\""; then
  echo "An identity named \"$NAME\" exists but is NOT valid for code signing, so bundle.sh will not use it." >&2
  echo "Usually the certificate is not trusted for code signing. To fix it, either:" >&2
  echo "  - open Keychain Access, find \"$NAME\", Get Info > Trust > Code Signing: Always Trust, or" >&2
  echo "  - delete it (Keychain Access, or: security delete-identity -c \"$NAME\") and run this script again." >&2
  exit 1
fi

echo "This will add a self-signed code-signing certificate \"$NAME\" to your login keychain."
echo "macOS may ask for your login password to allow the import."

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
PASS="nss-$(openssl rand -hex 8)"

cat > "$TMP/cert.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
CNF

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -keyout "$TMP/key.pem" -out "$TMP/cert.pem" -config "$TMP/cert.cnf" 2>/dev/null
# -legacy keeps the PKCS#12 readable by `security import` when OpenSSL 3 is installed.
openssl pkcs12 -export -legacy -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -out "$TMP/id.p12" -passout "pass:$PASS" 2>/dev/null \
  || openssl pkcs12 -export -inkey "$TMP/key.pem" -in "$TMP/cert.pem" -out "$TMP/id.p12" -passout "pass:$PASS"

security import "$TMP/id.p12" -k "$KEYCHAIN" -P "$PASS" -T /usr/bin/codesign
# Trust the certificate for code signing only, so `security find-identity -v -p codesigning` lists it.
# macOS shows a password prompt for this.
security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$TMP/cert.pem"
echo
echo "Done. Check with:  security find-identity -v -p codesigning"
echo "Now rebuild with ./scripts/bundle.sh. Allow Screen Recording once for the new build; later rebuilds keep it."

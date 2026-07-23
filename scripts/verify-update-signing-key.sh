#!/usr/bin/env bash
#
# Ensure the CI private update key matches the public key embedded in clients.

set -euo pipefail

PRIVATE_KEY_B64="${SHELLDECK_UPDATE_PRIVATE_KEY_PEM_BASE64:-}"
EXPECTED_PUBLIC_KEY_B64="${SHELLDECK_UPDATE_PUBLIC_KEY_BASE64:-}"

if [ -z "$PRIVATE_KEY_B64" ] || [ -z "$EXPECTED_PUBLIC_KEY_B64" ]; then
    echo "ERROR: Both update signing key variables are required" >&2
    exit 1
fi

TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT
KEY_PATH="$TEMP_DIR/private.pem"
PUBLIC_DER="$TEMP_DIR/public.der"

printf '%s' "$PRIVATE_KEY_B64" | base64 --decode > "$KEY_PATH"
chmod 600 "$KEY_PATH"
openssl pkey -in "$KEY_PATH" -pubout -outform DER -out "$PUBLIC_DER"

# An Ed25519 SubjectPublicKeyInfo DER value ends with the raw 32-byte key.
ACTUAL_PUBLIC_KEY_B64="$(
    tail -c 32 "$PUBLIC_DER" | base64 | tr -d '\r\n'
)"

if [ "$ACTUAL_PUBLIC_KEY_B64" != "$EXPECTED_PUBLIC_KEY_B64" ]; then
    echo "ERROR: Update signing private/public keys do not match" >&2
    exit 1
fi

echo "Update signing key pair matches"

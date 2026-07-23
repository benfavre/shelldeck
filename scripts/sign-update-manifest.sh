#!/usr/bin/env bash
#
# Add an Ed25519 signature to every platform entry in an update manifest.
#
# Usage:
#   SHELLDECK_UPDATE_PRIVATE_KEY_PEM_BASE64=... \
#     bash scripts/sign-update-manifest.sh unsigned.json signed.json
#
# Keep the canonical message in lockstep with
# `shelldeck_update::release_signing_message`.

set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 <unsigned-manifest.json> <signed-manifest.json>" >&2
    exit 2
fi

INPUT="$1"
OUTPUT="$2"
PRIVATE_KEY_B64="${SHELLDECK_UPDATE_PRIVATE_KEY_PEM_BASE64:-}"

if [ -z "$PRIVATE_KEY_B64" ]; then
    echo "ERROR: SHELLDECK_UPDATE_PRIVATE_KEY_PEM_BASE64 is required" >&2
    exit 1
fi
if [ ! -f "$INPUT" ]; then
    echo "ERROR: Manifest not found: $INPUT" >&2
    exit 1
fi
for command_name in jq openssl base64; do
    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "ERROR: Required command not found: $command_name" >&2
        exit 1
    fi
done

TEMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEMP_DIR"' EXIT
KEY_PATH="$TEMP_DIR/update-signing-key.pem"

printf '%s' "$PRIVATE_KEY_B64" | base64 --decode > "$KEY_PATH"
chmod 600 "$KEY_PATH"
openssl pkey -in "$KEY_PATH" -noout -check >/dev/null

jq -e '
    (.version | type == "string" and length > 0) and
    (.pub_date | type == "string" and length > 0) and
    (.platforms | type == "object" and length > 0)
' "$INPUT" >/dev/null

cp "$INPUT" "$TEMP_DIR/manifest.json"

while IFS= read -r platform; do
    version="$(jq -r '.version' "$TEMP_DIR/manifest.json")"
    pub_date="$(jq -r '.pub_date' "$TEMP_DIR/manifest.json")"
    url="$(jq -r --arg platform "$platform" '.platforms[$platform].url' "$TEMP_DIR/manifest.json")"
    sha256="$(jq -r --arg platform "$platform" '.platforms[$platform].sha256' "$TEMP_DIR/manifest.json")"
    size="$(jq -r --arg platform "$platform" '.platforms[$platform].size' "$TEMP_DIR/manifest.json")"

    for value in "$platform" "$version" "$pub_date" "$url" "$sha256" "$size"; do
        if [[ "$value" == *$'\n'* || "$value" == "null" || -z "$value" ]]; then
            echo "ERROR: Invalid manifest value for platform $platform" >&2
            exit 1
        fi
    done

    MESSAGE="$TEMP_DIR/message"
    SIGNATURE="$TEMP_DIR/signature"
    printf 'ShellDeck update manifest v1\nplatform=%s\npub_date=%s\nsha256=%s\nsize=%s\nurl=%s\nversion=%s\n' \
        "$platform" "$pub_date" "$sha256" "$size" "$url" "$version" > "$MESSAGE"
    openssl pkeyutl -sign -rawin -inkey "$KEY_PATH" -in "$MESSAGE" -out "$SIGNATURE"
    signature_b64="$(base64 < "$SIGNATURE" | tr -d '\r\n')"

    jq --arg platform "$platform" --arg signature "$signature_b64" \
        '.platforms[$platform].signature = $signature' \
        "$TEMP_DIR/manifest.json" > "$TEMP_DIR/next.json"
    mv "$TEMP_DIR/next.json" "$TEMP_DIR/manifest.json"
done < <(jq -r '.platforms | keys[]' "$TEMP_DIR/manifest.json")

jq -S -c . "$TEMP_DIR/manifest.json" > "$OUTPUT"
echo "Signed update manifest written to $OUTPUT"

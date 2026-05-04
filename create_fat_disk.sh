#!/bin/bash

IMG_DIR="target"
IMG_PATH="$IMG_DIR/fat.img"
SIZE_MB=64

dd if=/dev/zero of="$IMG_PATH" bs=1M count=$SIZE_MB
mkfs.fat -F 32 "$IMG_PATH"

STAGING=$(mktemp -d)

echo "HELLO FROM ROOT" > "$STAGING/README.TXT"
echo "VERSION 1.0"     > "$STAGING/VERSION.TXT"
mkdir -p "$STAGING/DATA"
echo "SOME DATA"       > "$STAGING/DATA/SAMPLE.DAT"
echo "MORE DATA"       > "$STAGING/DATA/TEST.BIN"

export MTOOLS_SKIP_CHECK=1
mcopy -i "$IMG_PATH" -s "$STAGING"/* ::/
rm -rf "$STAGING"

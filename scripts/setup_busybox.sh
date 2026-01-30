#!/bin/bash
# scripts/setup_busybox.sh
# Downloads and ingests a static busybox ternary as a "base image" manifest.

set -e

DEST_DIR="/tmp/velo_busybox"
MANIFEST_PATH="busybox.manifest"
BUSYBOX_URL="https://busybox.net/downloads/binaries/1.35.0-x86_64-linux-musl/busybox"

mkdir -p "$DEST_DIR/bin"

if [ ! -f "$DEST_DIR/bin/busybox" ]; then
    echo "[*] Downloading static busybox..."
    curl -L "$BUSYBOX_URL" -o "$DEST_DIR/bin/busybox"
    chmod +x "$DEST_DIR/bin/busybox"
fi

# Create symlinks for common tools
echo "[*] Creating symlinks..."
for cmd in sh ls cat cp mv rm mkdir echo id whoami; do
    ln -sf busybox "$DEST_DIR/bin/$cmd"
done

echo "[*] Ingesting busybox into Velo..."
velo ingest "$DEST_DIR" --output "$MANIFEST_PATH" --prefix "/"

echo "✓ Busybox base image created: $MANIFEST_PATH"
echo "  Usage: velo run --isolate --base $MANIFEST_PATH -- <command>"

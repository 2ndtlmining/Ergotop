#!/usr/bin/env bash
# Refreshes the embedded ergexplorer address-book snapshot.
# Keeps the committed snapshot if the download fails or looks incomplete.
set -euo pipefail

out="${1:-assets/addressbook-snapshot.json}"
url="https://api.ergexplorer.com/addressbook/getAddresses?offset=0&limit=5000&type=all&order=nameAsc&query=&testnet=0"
tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

if ! curl -fsS --max-time 30 "$url" -o "$tmp"; then
  echo "address book download failed; keeping $out"
  exit 0
fi

count="$(python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))["items"]))' "$tmp" 2>/dev/null || echo 0)"
if [ "$count" -lt 100 ]; then
  echo "address book download has $count items (< 100); keeping $out"
  exit 0
fi

mv "$tmp" "$out"
trap - EXIT
echo "address book snapshot refreshed: $count items"

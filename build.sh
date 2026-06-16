#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
cargo build --release --target wasm32-unknown-unknown
cp target/wasm32-unknown-unknown/release/tunerfish.wasm web/
echo "web/tunerfish.wasm: $(du -h web/tunerfish.wasm | cut -f1)"

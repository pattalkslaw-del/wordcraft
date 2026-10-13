#!/usr/bin/env bash
# LawCraft test gate: WordCraft's own full CI (fmt, clippy -D warnings, tests, assets, layers, wasm).
set -euo pipefail
cargo xtask ci

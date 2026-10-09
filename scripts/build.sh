#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir/frontend"
npm ci
npm run build
cd "$project_dir"
cargo build --locked --release "$@"

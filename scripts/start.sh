#!/usr/bin/env bash
set -euo pipefail
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$project_dir"
picsoc_binary="${PICSOC_BIN:-$project_dir/target/release/picsoc}"
if [[ ! -x "$picsoc_binary" ]]; then
  printf '找不到可执行程序：%s\n请先运行 ./scripts/build.sh，或设置 PICSOC_BIN。\n' "$picsoc_binary" >&2
  exit 1
fi
exec "$picsoc_binary" "$@"

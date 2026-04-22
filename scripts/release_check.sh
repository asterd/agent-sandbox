#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

need cargo
need rg

echo "[1/6] cargo test --workspace"
cargo test --workspace

echo "[2/6] cargo clippy --workspace -- -D warnings"
cargo clippy --workspace -- -D warnings

echo "[3/6] no todo!() in non-test Rust sources"
if rg -n 'todo!\(\)' crates/*/src --glob '!**/tests/**'; then
  echo "found todo!() in Rust sources" >&2
  exit 1
fi

echo "[4/6] required release docs exist"
test -f docs/spec-v1.md
test -f docs/api-http-v1.md
test -f docs/backends/docker.md
test -f docs/backends/podman.md
test -f docs/backends/gvisor.md
test -f docs/backends/firecracker.md
test -f BACKEND_GUIDE.md
test -f CHANGELOG.md

echo "[5/6] cargo audit available"
if ! cargo audit --ignore RUSTSEC-2023-0071 >/dev/null; then
  echo "cargo audit failed or is not installed" >&2
  exit 1
fi

echo "[6/6] release checks passed"

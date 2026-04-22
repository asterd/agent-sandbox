#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/export_runtime_state.sh --db sqlite:///var/lib/agentsandbox/agentsandbox.db --out /tmp/as-export
EOF
}

DB_URL=""
OUT_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --db) DB_URL="$2"; shift 2 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 1 ;;
  esac
done

if [[ -z "$DB_URL" || -z "$OUT_DIR" ]]; then
  usage
  exit 1
fi

DB_PATH="${DB_URL#sqlite:///}"
DB_PATH="${DB_PATH#sqlite://}"
if [[ ! -f "$DB_PATH" ]]; then
  echo "DB non trovato: $DB_PATH" >&2
  exit 1
fi

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "sqlite3 non trovato nel PATH" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
cp "$DB_PATH" "$OUT_DIR/agentsandbox.db"
sqlite3 -json "$DB_PATH" 'SELECT * FROM audit_log ORDER BY id ASC;' > "$OUT_DIR/audit_log.json"
sqlite3 -json "$DB_PATH" 'SELECT * FROM runtime_metadata ORDER BY key ASC;' > "$OUT_DIR/runtime_metadata.json"

echo "Export completato in $OUT_DIR"

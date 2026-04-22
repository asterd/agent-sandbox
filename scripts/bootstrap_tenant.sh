#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage:
  scripts/bootstrap_tenant.sh --db sqlite:///var/lib/agentsandbox/agentsandbox.db \
    --tenant-id default --api-key change-me [--quota-hourly 100] [--quota-concurrent 10]
EOF
}

DB_URL=""
TENANT_ID=""
API_KEY=""
QUOTA_HOURLY="100"
QUOTA_CONCURRENT="10"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --db) DB_URL="$2"; shift 2 ;;
    --tenant-id) TENANT_ID="$2"; shift 2 ;;
    --api-key) API_KEY="$2"; shift 2 ;;
    --quota-hourly) QUOTA_HOURLY="$2"; shift 2 ;;
    --quota-concurrent) QUOTA_CONCURRENT="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 1 ;;
  esac
done

if [[ -z "$DB_URL" || -z "$TENANT_ID" || -z "$API_KEY" ]]; then
  usage
  exit 1
fi

DB_PATH="${DB_URL#sqlite:///}"
DB_PATH="${DB_PATH#sqlite://}"
if [[ -z "$DB_PATH" || "$DB_PATH" == ":memory:" ]]; then
  echo "bootstrap_tenant.sh richiede un DB SQLite persistente" >&2
  exit 1
fi

mkdir -p "$(dirname "$DB_PATH")"

if ! command -v sqlite3 >/dev/null 2>&1; then
  echo "sqlite3 non trovato nel PATH" >&2
  exit 1
fi

API_KEY_HASH="$(printf '%s' "$API_KEY" | shasum -a 256 | awk '{print $1}')"
NOW="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

sqlite3 "$DB_PATH" <<EOF
INSERT INTO tenants (id, api_key_hash, quota_hourly, quota_concurrent, enabled, created_at)
VALUES ('$TENANT_ID', '$API_KEY_HASH', $QUOTA_HOURLY, $QUOTA_CONCURRENT, 1, '$NOW')
ON CONFLICT(id) DO UPDATE SET
  api_key_hash = excluded.api_key_hash,
  quota_hourly = excluded.quota_hourly,
  quota_concurrent = excluded.quota_concurrent,
  enabled = 1;

INSERT INTO tenant_usage (tenant_id, concurrent_in_use)
VALUES ('$TENANT_ID', 0)
ON CONFLICT(tenant_id) DO NOTHING;
EOF

echo "Tenant '$TENANT_ID' bootstrap completato su $DB_PATH"

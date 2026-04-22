# Internal Operations Guide

## Files Delivered

- Internal config example: `packaging/examples/agentsandbox.internal.toml`
- systemd unit: `packaging/systemd/agentsandbox.service`
- Tenant bootstrap: `scripts/bootstrap_tenant.sh`
- State export: `scripts/export_runtime_state.sh`

## Install

1. Build `agentsandbox-daemon` and copy the binary to `/opt/agentsandbox/bin/agentsandbox-daemon`.
2. Copy backend plugins to `/opt/agentsandbox/plugins`.
3. Copy `packaging/examples/agentsandbox.internal.toml` to `/etc/agentsandbox/agentsandbox.internal.toml`.
4. Create runtime directories:
   - `/var/lib/agentsandbox`
   - `/var/log/agentsandbox`
5. Bootstrap the first tenant before the first service start:

```bash
scripts/bootstrap_tenant.sh \
  --db sqlite:///var/lib/agentsandbox/agentsandbox.db \
  --tenant-id default \
  --api-key change-me
```

## Start

```bash
sudo cp packaging/systemd/agentsandbox.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now agentsandbox
```

The daemon refuses startup when:

- `auth.mode = "api_key"` and no tenant is active
- the service listens on a non-local address without `auth.mode = "api_key"`
- limits are invalid (`max_ttl_seconds`, `default_timeout_ms`, `max_file_bytes` <= 0)

## Runtime Checks

```bash
curl -sS http://127.0.0.1:7847/v1/runtime-info
curl -sS -H "X-API-Key: <KEY>" http://127.0.0.1:7847/v1/admin/tenants/default/usage
```

## Backup And Export

Create a portable copy of the DB plus JSON exports:

```bash
scripts/export_runtime_state.sh \
  --db sqlite:///var/lib/agentsandbox/agentsandbox.db \
  --out /tmp/agentsandbox-export
```

The daemon also runs periodic cleanup for:

- old `audit_log` rows according to `audit.retain_days`
- stale `rate_limit_windows`
- SQLite `VACUUM` on persistent databases

## Upgrade And Rollback

1. Export runtime state.
2. Stop the service.
3. Replace the daemon binary and plugin files.
4. Start the service and check `/v1/runtime-info`.
5. If rollback is needed, restore the previous binary and the exported DB copy, then restart the service.

## Reverse Proxy Notes

Terminate TLS on the reverse proxy and forward:

- `X-Forwarded-For`
- `X-Forwarded-Proto`
- `X-Request-Id`

Keep `security.trusted_proxy_headers = true` only when the proxy is under platform control.

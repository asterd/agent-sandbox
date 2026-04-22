# Internal Troubleshooting

## Daemon Does Not Start

- Check `journalctl -u agentsandbox`.
- Verify `auth.mode = "api_key"` has at least one enabled tenant.
- Verify the DB URL points to a writable SQLite file.

## No Backend Available

- Confirm plugin binaries are in `backends.search_dirs`.
- Validate backend host requirements:
  - Docker: daemon/socket reachable
  - Podman: socket reachable
- Check `/v1/runtime-info` for discovered backends.

## Upload/Download Fails

- Ensure the client sends `X-Lease-Token`.
- Verify the file size is below `limits.max_file_bytes`.
- If the backend returns `NOT_SUPPORTED`, the capability is not implemented for that backend.

## Concurrent Quota Unexpected

- Inspect `/v1/admin/tenants/<tenant>/usage`.
- Restarting the daemon reconciles `tenant_usage` from active sandbox rows.

## Audit Growth

- Tune `audit.retain_days`.
- Run `scripts/export_runtime_state.sh` before manual archival.

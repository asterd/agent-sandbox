# Internal Smoke Checklist

Target environment:

- single host
- daemon as service
- Docker or Podman backend
- 2 tenants
- Python SDK and TypeScript SDK

Checklist:

1. `GET /v1/runtime-info` returns `config_profile`, `auth_mode`, backends, and DB path.
2. `scripts/bootstrap_tenant.sh` creates two active tenants.
3. Tenant A creates a sandbox and uploads a file.
4. Tenant A consumes `exec?stream=1` and receives `started`, `stdout`, `stderr`, `completed`.
5. Tenant A downloads the generated file and destroys the sandbox.
6. Tenant B cannot inspect usage for Tenant A.
7. `GET /v1/admin/tenants/<tenant>/usage` shows hourly and concurrent counts.
8. A request with `privileged=true` is rejected when `security.allow_privileged_extensions = false`.
9. Export script produces `agentsandbox.db`, `audit_log.json`, and `runtime_metadata.json`.

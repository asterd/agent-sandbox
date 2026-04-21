# Spec v1

The public sandbox contract is `sandbox.ai/v1`.

Minimal example:

```yaml
apiVersion: sandbox.ai/v1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  ttlSeconds: 300
```

Additional fields available in `v1`:

- `runtime.version`
- `resources.timeoutMs`
- `network.egress.mode`
- `scheduling.backend`
- `scheduling.priority`
- `storage.volumes`
- `observability.auditLevel`
- `observability.metricsEnabled`

## `network.egress`

`network.egress.allow` accepts hostnames only.

Current limits:

- resolution happens once at sandbox creation time
- direct IPs are rejected
- wildcard hostnames are rejected
- paths such as `example.com/api` are rejected
- if the runtime image cannot enforce the allowlist, sandbox creation fails closed

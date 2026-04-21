# Spec v1beta1

`sandbox.ai/v1beta1` extends `v1alpha1` without removing or renaming fields.

New optional fields:

- `spec.runtime.version`
- `spec.resources.timeoutMs`
- `spec.network.egress.mode`
- `spec.scheduling.backend`
- `spec.scheduling.priority`
- `spec.storage.volumes`
- `spec.observability.auditLevel`
- `spec.observability.metricsEnabled`

Example:

```yaml
apiVersion: sandbox.ai/v1beta1
kind: Sandbox
metadata:
  name: review-job
spec:
  runtime:
    preset: python
    version: "3.12"
  resources:
    cpuMillicores: 1000
    memoryMb: 512
    diskMb: 1024
    timeoutMs: 30000
  network:
    egress:
      allow:
        - pypi.org
      denyByDefault: true
      mode: proxy
  scheduling:
    backend: docker
    preferWarm: false
    priority: normal
  storage:
    volumes: []
  observability:
    auditLevel: basic
    metricsEnabled: false
  ttlSeconds: 300
```

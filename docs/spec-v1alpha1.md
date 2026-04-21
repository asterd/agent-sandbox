# Spec v1alpha1

The public sandbox contract is `sandbox.ai/v1alpha1`.

Anything not expressed here is not part of the API.

## Top-level shape

```yaml
apiVersion: sandbox.ai/v1alpha1
kind: Sandbox
metadata:
  name: optional-name
  labels:
    team: infra
spec:
  runtime:
    preset: python
  ttlSeconds: 300
```

## Fields

### `apiVersion`

Must be exactly:

```text
sandbox.ai/v1alpha1
```

### `kind`

Must be:

```text
Sandbox
```

### `metadata`

Optional metadata.

- `metadata.name`
- `metadata.labels`

### `spec.runtime`

Runtime selection.

Allowed patterns:

- `runtime.preset`
- `runtime.image`
- `runtime.preset=custom` only together with explicit `runtime.image`

Supported presets:

- `python` -> `python:3.12-slim`
- `node` -> `node:20-slim`
- `rust` -> `rust:1.77-slim`
- `shell` -> `ubuntu:24.04`
- `custom` -> requires `runtime.image`

Optional runtime fields:

- `runtime.env`: non-secret environment variables
- `runtime.workingDir`: guest working directory, default `/workspace`

### `spec.resources`

Optional resource limits.

- `cpuMillicores`, default `1000`
- `memoryMb`, default `512`
- `diskMb`, default `1024`

### `spec.network.egress`

Optional egress policy.

```yaml
network:
  egress:
    allow:
      - pypi.org
      - files.pythonhosted.org
    denyByDefault: true
```

Rules:

- hostnames only
- no IPs
- no wildcards
- no paths
- `denyByDefault` defaults to `true`

### `spec.secrets`

Optional guest environment bindings for host secrets.

```yaml
secrets:
  - name: OPENAI_API_KEY
    valueFrom:
      envRef: OPENAI_API_KEY
  - name: SERVICE_TOKEN
    valueFrom:
      file: /tmp/service-token.txt
```

Rules:

- each `valueFrom` must contain exactly one of `envRef` or `file`
- missing host values fail compilation
- secret values are resolved by the daemon and never exposed by the SDKs

### `spec.ttlSeconds`

Optional sandbox lifetime in seconds.

Default:

```text
300
```

### `spec.scheduling`

Currently accepted for forward compatibility.

```yaml
scheduling:
  preferWarm: false
```

`preferWarm` is ignored in `v1alpha1`.

## Full example

```yaml
apiVersion: sandbox.ai/v1alpha1
kind: Sandbox
metadata:
  name: review-job
  labels:
    app: agentsandbox
spec:
  runtime:
    preset: python
    env:
      PYTHONDONTWRITEBYTECODE: "1"
    workingDir: /workspace
  resources:
    cpuMillicores: 1000
    memoryMb: 512
    diskMb: 1024
  network:
    egress:
      allow:
        - pypi.org
        - files.pythonhosted.org
      denyByDefault: true
  secrets:
    - name: API_TOKEN
      valueFrom:
        envRef: HOST_API_TOKEN
  ttlSeconds: 900
  scheduling:
    preferWarm: false
```

## Known limits of `network.egress` in `v1alpha1`

- DNS resolution happens once at sandbox creation time
- DNS rebinding is not prevented
- wildcard hostnames like `*.example.com` are rejected
- direct IPs in `egress.allow` are rejected
- Docker enforcement uses `iptables` inside the guest; if the runtime image cannot apply the policy, sandbox creation fails instead of silently opening egress

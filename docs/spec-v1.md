# Spec v1

The public contract is `sandbox.ai/v1`.

## Full YAML Example

```yaml
apiVersion: sandbox.ai/v1
kind: Sandbox
metadata:
  labels:
    app: example
spec:
  runtime:
    preset: python
    version: "3.12"
    workingDir: /workspace
    env:
      LOG_LEVEL: info
  resources:
    cpuMillicores: 500
    memoryMb: 256
    diskMb: 512
    timeoutMs: 30000
  network:
    egress:
      mode: proxy
      allow:
        - pypi.org
      denyByDefault: true
  secrets:
    - name: OPENAI_API_KEY
      valueFrom:
        envRef: OPENAI_API_KEY
  ttlSeconds: 300
  scheduling:
    backend: docker
    preferWarm: false
    priority: normal
  extensions:
    docker:
      hostConfig:
        capDrop: ["ALL"]
  storage:
    volumes: []
  observability:
    auditLevel: basic
    metricsEnabled: false
```

## Full JSON Example

```json
{
  "apiVersion": "sandbox.ai/v1",
  "kind": "Sandbox",
  "metadata": {
    "labels": {
      "app": "example"
    }
  },
  "spec": {
    "runtime": {
      "preset": "python",
      "version": "3.12",
      "workingDir": "/workspace",
      "env": {
        "LOG_LEVEL": "info"
      }
    },
    "resources": {
      "cpuMillicores": 500,
      "memoryMb": 256,
      "diskMb": 512,
      "timeoutMs": 30000
    },
    "network": {
      "egress": {
        "mode": "proxy",
        "allow": ["pypi.org"],
        "denyByDefault": true
      }
    },
    "secrets": [
      {
        "name": "OPENAI_API_KEY",
        "valueFrom": {
          "envRef": "OPENAI_API_KEY"
        }
      }
    ],
    "ttlSeconds": 300,
    "scheduling": {
      "backend": "docker",
      "preferWarm": false,
      "priority": "normal"
    },
    "extensions": {
      "docker": {
        "hostConfig": {
          "capDrop": ["ALL"]
        }
      }
    },
    "storage": {
      "volumes": []
    },
    "observability": {
      "auditLevel": "basic",
      "metricsEnabled": false
    }
  }
}
```

## Field Reference

| Field | Type | Required | Default | Notes |
| --- | --- | --- | --- | --- |
| `apiVersion` | string | yes | none | Must be `sandbox.ai/v1`. |
| `kind` | string | yes | none | Must be `Sandbox`. |
| `metadata.labels` | object<string,string> | no | `{}` | Free-form labels copied to the IR. |
| `spec.runtime.preset` | enum | yes* | none | One of `python`, `node`, `rust`, `shell`, `custom`. Required unless `runtime.image` is set. |
| `spec.runtime.image` | string | yes* | preset-derived | Explicit container image. Required for `preset=custom`. |
| `spec.runtime.version` | string | no | preset default | Example: `3.12`, `20`, `1.77`. |
| `spec.runtime.workingDir` | string | no | `/workspace` | Working directory inside the sandbox. |
| `spec.runtime.env` | object<string,string> | no | `{}` | Plain environment variables. |
| `spec.resources.cpuMillicores` | integer | no | `1000` | CPU budget in millicores. |
| `spec.resources.memoryMb` | integer | no | `512` | Memory limit in MB. |
| `spec.resources.diskMb` | integer | no | `1024` | Disk budget in MB. |
| `spec.resources.timeoutMs` | integer | no | `30000` | Default exec timeout in milliseconds. |
| `spec.network.egress.mode` | enum | no | `proxy` | `none`, `proxy`, `passthrough`. |
| `spec.network.egress.allow` | array<string> | no | `[]` | Hostnames only, no IPs, no wildcards, no paths. |
| `spec.network.egress.denyByDefault` | boolean | no | `true` | Fail closed when the backend cannot enforce rules. |
| `spec.secrets[].name` | string | no | none | Environment variable name exposed inside the sandbox. |
| `spec.secrets[].valueFrom.envRef` | string | conditional | none | Host env var to resolve at compile time. |
| `spec.secrets[].valueFrom.file` | string | conditional | none | Host file path to read at compile time. |
| `spec.ttlSeconds` | integer | no | `300` | Sandbox lifetime. |
| `spec.scheduling.backend` | string | no | scheduler choice | Required when `spec.extensions` is present. |
| `spec.scheduling.preferWarm` | boolean | no | `false` | Hint for warm-pool capable backends. |
| `spec.scheduling.priority` | enum | no | unset | `low`, `normal`, `high`. |
| `spec.extensions` | object | no | unset | Backend-specific options validated by backend schema. |
| `spec.storage.volumes` | array<object> | no | `[]` | Reserved for backend-specific storage attachments. |
| `spec.observability.auditLevel` | enum | no | unset | `none`, `basic`, `full`. |
| `spec.observability.metricsEnabled` | boolean | no | `false` | Enables backend metrics hints in the IR. |

\* `runtime.preset` and `runtime.image` are jointly required: one of them must resolve to an image.

## Validation Rules

- Unknown fields are rejected.
- `spec.extensions` requires `spec.scheduling.backend`.
- `extensions.docker.hostConfig.networkMode` and `extensions.podman.hostConfig.networkMode` are forbidden: use `spec.network.egress`.
- `extensions.firecracker.vsock` is reserved for internal exec transport.
- Secret values are resolved at compile time but never serialized into the stored IR.

## `network.egress`

`network.egress.allow` accepts hostnames only.

Current limits:

- direct IPs are rejected
- wildcard hostnames are rejected
- hostnames with paths such as `example.com/api` are rejected
- backends fail closed when they cannot safely enforce the allowlist

# Firecracker Backend

`firecracker` is reserved in the public contract but is not shipped as a production-ready backend in this repository snapshot.

## Current Status

- no `agentsandbox-backend-firecracker` crate is present yet
- `spec.extensions.firecracker.vsock` is already reserved and rejected by the compile pipeline
- this document exists to keep the public release docs explicit about the current gap instead of implying support

## Planned Shape

Expected daemon configuration:

```toml
[backends]
enabled = ["firecracker"]

[backends.firecracker]
kernel_image = "/var/lib/agentsandbox/vmlinux"
rootfs_image = "/var/lib/agentsandbox/rootfs.ext4"
```

Expected spec routing:

```yaml
apiVersion: sandbox.ai/v1
kind: Sandbox
metadata: {}
spec:
  runtime:
    preset: python
  scheduling:
    backend: firecracker
```

## Release Guidance

Do not enable `firecracker` in default configs until a dedicated backend crate exists and passes the conformance suite.

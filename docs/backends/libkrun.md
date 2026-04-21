# libkrun Backend

Backend `libkrun` implementato in modo pragmatico come variante del runtime Podman `krun`,
come previsto dalla nota di fallback nel roadmap.

## Piattaforme

- Linux con `/dev/kvm`
- Podman socket raggiungibile
- runtime `krun` disponibile

## Configurazione

```toml
[backends]
enabled = ["libkrun"]

[backends.libkrun]
socket = "/run/user/1000/podman/podman.sock"
runtime = "krun"
```

## Note operative

- Se `/dev/kvm` manca, `health_check()` ritorna `Unavailable` con messaggio esplicito.
- L'esecuzione delega al backend container esistente con `runtime=krun`.

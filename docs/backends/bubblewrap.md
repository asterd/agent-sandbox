# Bubblewrap Backend

Backend `bubblewrap` per sviluppo locale rootless.

## Piattaforme

- Linux: usa `bwrap`
- macOS: usa `sandbox-exec` come fallback compatibile con la stessa API

## Configurazione

```toml
[backends]
enabled = ["bubblewrap"]

[backends.bubblewrap]
bwrap_path = "bwrap"
rootfs_base = "/tmp/agentsandbox-bubblewrap"
```

## Note operative

- `health_check()` fallisce con un messaggio esplicito se `bwrap` o `sandbox-exec` non sono disponibili.
- I workspace temporanei vengono creati sotto `rootfs_base` e rimossi da `destroy()`.
- Su macOS il backend espone solo il profilo base; mount extra e argomenti raw non sono supportati.

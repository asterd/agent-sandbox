# nsjail Backend

Backend `nsjail` per host Linux con namespace, seccomp e cgroup.

## Piattaforme

- Solo Linux

## Configurazione

```toml
[backends]
enabled = ["nsjail"]

[backends.nsjail]
nsjail_path = "nsjail"
chroot_base = "/tmp/agentsandbox-nsjail"
```

## Privilegi

`nsjail` puo' richiedere user namespaces o capability elevate a seconda dell'host.
Se l'ambiente non lo consente, `health_check()` ritorna `Unavailable`.

## Note operative

- `destroy()` rimuove il workspace temporaneo.
- Le extension supportate sono validate sotto `extensions.nsjail`.

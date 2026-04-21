# Spec v1alpha1

## Limiti noti di `network.egress` in v1alpha1

- La risoluzione DNS avviene una volta sola alla creazione della sandbox. Se un hostname cambia IP dopo la creazione, il nuovo IP non entra nella allowlist.
- Il DNS rebinding non e' prevenuto: un host esterno puo' cambiare risposta dopo la risoluzione iniziale.
- Le wildcard negli hostname come `*.example.com` non sono supportate e producono un errore di validazione.
- Gli IP diretti in `egress.allow` non sono supportati e producono un errore di validazione. Usa hostname espliciti.
- L'enforcement Docker v1alpha1 usa `iptables` dentro al guest e richiede che l'immagine abbia il comando `iptables` disponibile. Se `network.egress` richiede una allowlist ma il runtime non puo' applicarla, la creazione della sandbox fallisce invece di degradare a rete aperta.

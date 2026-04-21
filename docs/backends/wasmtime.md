# Wasmtime Backend

Backend `wasmtime` per esecuzione ultra-leggera e portabile.

## Limiti attuali

Questa fase introduce un **compat runner minimo** coerente con l'API backend corrente:

- supporta `echo ...`
- supporta `echo ... >&2`
- supporta `exit N`
- supporta `python -c 'print(expr)'` per espressioni aritmetiche semplici

Il wiring per moduli WASM esterni e runtime Python/Node completi e' predisposto in configurazione,
ma non e' ancora il path di esecuzione di default.

## Configurazione

```toml
[backends]
enabled = ["wasmtime"]

[backends.wasmtime]
python_wasm_path = "./python.wasm"
node_wasm_path = "./node.wasm"
```

## Quando usarlo

- test leggeri e deterministici
- ambienti senza Docker
- casi in cui serve un backend self-contained

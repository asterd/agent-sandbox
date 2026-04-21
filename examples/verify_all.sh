#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXAMPLES_DIR="$ROOT_DIR/examples"
TMP_OUT="${TMPDIR:-/tmp}/agentsandbox-examples.out"
PASS=0
FAIL=0
SKIP=0

find_python() {
    local candidates=(
        "$ROOT_DIR/sdks/python/.venv/bin/python"
        python3
        python
    )
    local candidate
    for candidate in "${candidates[@]}"; do
        if command -v "$candidate" >/dev/null 2>&1; then
            echo "$candidate"
            return 0
        fi
    done
    return 1
}

check_daemon() {
    if ! curl -sf http://127.0.0.1:7847/v1/health >/dev/null; then
        echo "Daemon non raggiungibile. Avvia con: cargo run -p agentsandbox-daemon"
        exit 1
    fi
    echo "Daemon raggiungibile"
}

run_example() {
    local name="$1"
    local dir="$2"
    local cmd="$3"

    printf "  %s... " "$name"
    if (
        cd "$EXAMPLES_DIR/$dir"
        eval "$cmd"
    ) >"$TMP_OUT" 2>&1; then
        echo "OK"
        PASS=$((PASS + 1))
    else
        echo "FAIL"
        FAIL=$((FAIL + 1))
        sed 's/^/    /' "$TMP_OUT" | head -15
    fi
}

skip_example() {
    local name="$1"
    local reason="$2"
    printf "  %s... SKIP (%s)\n" "$name" "$reason"
    SKIP=$((SKIP + 1))
}

echo "=== AgentSandbox Examples Verification ==="
check_daemon
echo ""

PYTHON_BIN="$(find_python)" || {
    echo "Interpreter Python non trovato"
    exit 1
}

run_example "01-hello-sandbox" "01-hello-sandbox" "$PYTHON_BIN run.py"
run_example "04-multi-backend-demo" "04-multi-backend-demo" "$PYTHON_BIN demo.py"

if [[ -n "${ANTHROPIC_API_KEY:-}" ]]; then
    run_example "02-code-review-agent" "02-code-review-agent" "$PYTHON_BIN agent.py sample_code/buggy_script.py"

    if [[ -x "$EXAMPLES_DIR/03-dependency-auditor/node_modules/.bin/tsx" ]]; then
        run_example \
            "03-dependency-auditor" \
            "03-dependency-auditor" \
            "npm run start -- sample/package.json"
    else
        skip_example "03-dependency-auditor" "npm install non eseguito"
    fi
else
    skip_example "02-code-review-agent" "ANTHROPIC_API_KEY non settata"
    skip_example "03-dependency-auditor" "ANTHROPIC_API_KEY non settata"
fi

echo ""
echo "Risultati: $PASS OK  $FAIL FAIL  $SKIP SKIP"
[[ $FAIL -eq 0 ]]

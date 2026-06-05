#!/usr/bin/env bash
# Ollama generation watchdog.
#
# A wedged Ollama runner still answers API/CLI calls (`ollama ps` works) while
# every generation hangs forever — so probing /api/tags proves nothing. This
# probes with a real 1-token generation and restarts the service if it doesn't
# answer in time. Side bonus: the probe keeps the model resident, so cold
# starts become rare.
#
# Installed by deploy/ollama-watchdog/README.md; runs from a systemd timer.

set -u

MODEL="${OLLAMA_WATCHDOG_MODEL:-qwen3.6:27b}"
URL="${OLLAMA_WATCHDOG_URL:-http://localhost:11434}"
# Generous: a cold load of a large model can legitimately take a couple of
# minutes. If one token can't come back in this long, it's wedged.
TIMEOUT_SECS="${OLLAMA_WATCHDOG_TIMEOUT:-180}"

probe() {
  curl -sf --max-time "$TIMEOUT_SECS" "$URL/api/chat" -d "{
    \"model\": \"$MODEL\",
    \"messages\": [{\"role\": \"user\", \"content\": \"ping\"}],
    \"stream\": false,
    \"options\": {\"num_predict\": 1}
  }" > /dev/null
}

if probe; then
  exit 0
fi

echo "ollama-watchdog: generation probe failed (model=$MODEL, timeout=${TIMEOUT_SECS}s) — restarting ollama"
systemctl restart ollama

# Confirm recovery so the journal shows the outcome.
sleep 10
if probe; then
  echo "ollama-watchdog: recovered after restart"
else
  echo "ollama-watchdog: still failing after restart — needs attention (check nvidia-smi / journalctl -u ollama)"
  exit 1
fi

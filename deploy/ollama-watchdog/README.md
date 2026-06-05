# Ollama watchdog

Restarts Ollama when generation wedges (API still answers, generations hang —
the failure mode where `ollama ps` works but chat never responds).

Probes a real 1-token generation every 5 minutes; on failure runs
`systemctl restart ollama` and logs the outcome to the journal. The probe also
keeps the model warm, so cold starts become rare.

## Install (on the machine running Ollama)

```bash
cd ~/dev/episteme/deploy/ollama-watchdog
sudo cp ollama-watchdog.sh /usr/local/bin/ && sudo chmod +x /usr/local/bin/ollama-watchdog.sh
sudo cp ollama-watchdog.service ollama-watchdog.timer /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now ollama-watchdog.timer
```

## Check it

```bash
systemctl list-timers ollama-watchdog.timer   # next/last run
journalctl -u ollama-watchdog -n 20           # probe results / restarts
```

## Tune

Override via a drop-in (`systemctl edit ollama-watchdog.service`) setting
environment variables:

- `OLLAMA_WATCHDOG_MODEL` — probe model (default `qwen3.6:27b`)
- `OLLAMA_WATCHDOG_URL` — server URL (default `http://localhost:11434`)
- `OLLAMA_WATCHDOG_TIMEOUT` — seconds before declaring it wedged (default 180)

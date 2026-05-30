# Deploying the collector on a Linux server (Ubuntu 24.04)

This runs the headless `collector` binary as the same user that runs Claude Code,
uploading this machine's usage data to Supabase under a **distinct** prefix so it
never overwrites the Windows machine's data.

## 0. Prerequisites

- Claude Code is already installed and logged in on the server (so
  `~/.claude/.credentials.json` exists and `~/.claude/projects/` is being written).
- You can build Rust on the box: install the toolchain and a linker:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
  sudo apt-get update && sudo apt-get install -y build-essential
  ```

## 1. Clone and build

```bash
git clone <your-fork-or-repo-url> ~/claude-usage-tray
cd ~/claude-usage-tray
cargo build --release --bin collector
# binary at: ~/claude-usage-tray/target/release/collector
```

If the build fails on TLS, confirm `ureq` resolved to rustls (it does by default
with `features = ["json"]`); only if it pulled native-tls would you need
`sudo apt-get install -y libssl-dev pkg-config`.

## 2. Create the secrets file (distinct prefix!)

Put the `.env` where the service's working directory will be, NOT inside the git
work tree's tracked area (it is gitignored, but keep it isolated anyway):

```bash
mkdir -p ~/.config/claude-usage-tray
cat > ~/.config/claude-usage-tray/.env <<'EOF'
SUPABASE_URL=https://YOUR_PROJECT_REF.supabase.co
SUPABASE_SERVICE_ROLE_KEY=YOUR_KEY
SUPABASE_BUCKET=usage-tracker
SUPABASE_USER_PREFIX=borgi-linux
EOF
chmod 600 ~/.config/claude-usage-tray/.env
```

- **Do NOT copy the Windows `.env` verbatim** — it sets `SUPABASE_USER_PREFIX=borgi`
  and would make the server clobber the Windows objects every cycle.
- Prefer a Supabase **Storage-scoped** key over the full `service_role` key if you
  can create one; `service_role` bypasses Row Level Security and could delete every
  prefix in the bucket if the box is compromised.

## 3. Smoke test

```bash
cd ~/.config/claude-usage-tray
~/claude-usage-tray/target/release/collector --once --log-level debug
```

The collector loads `.env` from its **current working directory**, so run it from
`~/.config/claude-usage-tray`. Expected: a `cycle complete` log line, and
`borgi-linux/cache.parquet` (+ `caps.json`, `calibration_log.parquet`) appearing in
the Supabase Storage dashboard.

## 4. Run as a systemd user service (survives reboot)

Create `~/.config/systemd/user/claude-collector.service`:

```ini
[Unit]
Description=Claude Code usage collector
After=network-online.target

[Service]
Type=simple
WorkingDirectory=%h/.config/claude-usage-tray
ExecStart=%h/claude-usage-tray/target/release/collector --interval 120
Restart=on-failure
RestartSec=30

[Install]
WantedBy=default.target
```

Enable it and turn on **linger** so it runs without an active login session
(i.e. survives reboot and logout):

```bash
systemctl --user daemon-reload
systemctl --user enable --now claude-collector.service
loginctl enable-linger "$USER"
```

Check status / logs:

```bash
systemctl --user status claude-collector.service
journalctl --user -u claude-collector.service -f
```

## Notes

- The collector is independent of your `tmux new -s claude1` session; you do not
  need tmux open for it to run.
- The poll cadence is `--interval` (default 120s). Keep it >= 60s to respect the
  usage endpoint's ~1 request/minute limit (the binary clamps any lower value up
  to 60s).
- Token freshness: because you actively run Claude Code on the server, it keeps
  `~/.claude/.credentials.json` current. If the token ever expires, the collector
  logs a warning, skips the poll, and still uploads local turns that cycle.

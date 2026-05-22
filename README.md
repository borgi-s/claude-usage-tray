# claude-usage-tray

Native Windows tray widget for monitoring [Claude Code](https://docs.claude.com/en/docs/claude-code) token usage against Anthropic's 5h and weekly rate limits.

**Status:** Stage 1 — single-shot CLI. The system tray UI and dashboard window are upcoming stages.

## Why

The official Claude Code CLI shows your usage on demand, but it's nice to have a tray icon that surfaces it constantly. This project is the Rust port / replacement of the Python+Streamlit [claude-usage-tracker](https://github.com/borgi-s/claude-usage-tracker), built natively for Windows.

## Requirements

- Windows 10/11
- An active Claude Code login (this app reads `~/.claude/.credentials.json` that Claude Code maintains)

## Install (Stage 1)

Build from source — pre-built binaries land at Stage 3 once there's a tray icon worth shipping:

```powershell
cargo build --release
```

The binary appears at `target\release\claude-usage-tray.exe`.

## Usage

```powershell
.\target\release\claude-usage-tray.exe --once
```

Output:
```
5h: 56% (resets in 2h 13m)
7d: 56% (resets in 1d 21h)
sub: pro / tier: default_claude_ai
```

## Roadmap

See `docs/superpowers/specs/2026-05-22-rust-tray-widget-design.md` for the full 8-stage plan.

## License

MIT — see `LICENSE`.

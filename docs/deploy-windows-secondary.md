# Flipping the Windows tray to secondary mode (Phase 3)

By default the Windows tray polls Anthropic's usage API every ~2 minutes. Once the
Linux server is the always-on primary poller (Phase 1) and the Streamlit viewer
merges both machines (Phase 2), the Windows machine should STOP polling so the two
machines don't contend on the ~1 req/min/account rate limit.

In secondary mode the Windows tray:
- reads account-wide caps from `borgi-linux/caps.json` in Supabase (kept fresh by
  the Linux poller) to drive the tray icon / widget / live-banner %,
- uploads only its own local turns (`borgi/cache.parquet`),
- never calls the usage API.

## How to switch

Add ONE line to the Windows `.env` (next to the existing `SUPABASE_*` vars):

```
SUPABASE_CAPS_PREFIX=borgi-linux
```

Then restart the tray. On startup the log shows:

```
live caps source: cloud (secondary mode; NOT polling the usage API)
```

## How to switch back

Remove (or comment out) `SUPABASE_CAPS_PREFIX` and restart the tray. It returns to
polling the usage API and uploading all three objects under its own prefix.

## Prerequisites / notes

- The Linux collector must be running and writing `borgi-linux/caps.json` (Phase 1),
  otherwise the tray's % readout will be empty (it shows the gray "no data" icon).
- `SUPABASE_URL` / `SUPABASE_SERVICE_ROLE_KEY` / `SUPABASE_BUCKET` must still be set
  (the tray needs the Supabase client to read caps and to upload its turns).
- Secondary mode does not need a fresh OAuth token: if the laptop's token is expired
  at launch, the tray still starts (it logs a warning and shows "unknown" plan/tier
  in the tooltip). It will pick up real creds again automatically once you run Claude
  Code on the laptop and restart, or whenever you switch back to primary mode.
- Keep `SUPABASE_USER_PREFIX=borgi` on Windows unchanged — that's still where Windows
  uploads its own turns.

//! Stage 7: best-effort upload of cache + calibration log + caps to Supabase
//! Storage, so the polars cloud viewer reads them unchanged. See
//! docs/superpowers/specs/2026-05-23-stage-7-supabase-sync-design.md.

pub mod config;
pub mod export;
pub mod storage;

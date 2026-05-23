//! Serialize in-memory/on-disk state into parquet + JSON byte buffers that the
//! polars cloud viewer reads. Schemas mirror the Python project exactly.

use crate::api::credentials::Credentials;
use crate::data::parser::Turn;
use crate::log::calibration::CalibrationSample;
use crate::shared::snapshot::AppSnapshot;
use anyhow::Result;
use serde::Serialize;
use arrow::array::{ArrayRef, BooleanArray, Float64Array, Int64Array, StringArray, TimestampMillisecondArray};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use std::sync::Arc;

/// Serialize the cache (one row per turn) to parquet bytes.
pub fn cache_parquet(turns: &[Turn]) -> Result<Vec<u8>> {
    let schema = Arc::new(Schema::new(vec![
        Field::new("timestamp", DataType::Utf8, false),
        Field::new("session_id", DataType::Utf8, false),
        Field::new("subagent_id", DataType::Utf8, true),
        Field::new("is_subagent", DataType::Boolean, false),
        Field::new("project_cwd", DataType::Utf8, false),
        Field::new("model", DataType::Utf8, false),
        Field::new("version", DataType::Utf8, false),
        Field::new("input_tokens", DataType::Int64, false),
        Field::new("output_tokens", DataType::Int64, false),
        Field::new("cache_creation_input_tokens", DataType::Int64, false),
        Field::new("cache_read_input_tokens", DataType::Int64, false),
        Field::new("source_file", DataType::Utf8, false),
        Field::new("is_rate_limit_error", DataType::Boolean, false),
    ]));

    let columns: Vec<ArrayRef> = vec![
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.ts.to_rfc3339()))),
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.session_id.clone()))),
        Arc::new(StringArray::from(turns.iter().map(|t| t.subagent_id.clone()).collect::<Vec<Option<String>>>())),
        Arc::new(BooleanArray::from(turns.iter().map(|t| t.is_subagent).collect::<Vec<bool>>())),
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.project_cwd.clone()))),
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.model.clone()))),
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.version.clone()))),
        // Token counts are always small (<10^9); i64::MAX is ~9.2*10^18, so the cast never wraps.
        Arc::new(Int64Array::from_iter_values(turns.iter().map(|t| t.input_tokens as i64))),
        Arc::new(Int64Array::from_iter_values(turns.iter().map(|t| t.output_tokens as i64))),
        Arc::new(Int64Array::from_iter_values(turns.iter().map(|t| t.cache_creation_input_tokens as i64))),
        Arc::new(Int64Array::from_iter_values(turns.iter().map(|t| t.cache_read_input_tokens as i64))),
        Arc::new(StringArray::from_iter_values(turns.iter().map(|t| t.source_file.to_string_lossy().into_owned()))),
        Arc::new(BooleanArray::from(turns.iter().map(|t| t.is_rate_limit_error).collect::<Vec<bool>>())),
    ];

    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    write_parquet(schema, &batch)
}

/// Serialize the calibration log (one row per sample) to parquet bytes. Columns
/// the cloud viewer does not read (burns + per-window token aggregates) are
/// emitted as all-null; see the Stage 7 spec.
pub fn calibration_log_parquet(samples: &[CalibrationSample]) -> Result<Vec<u8>> {
    let n = samples.len();

    let schema = Arc::new(Schema::new(vec![
        Field::new("sampled_at", DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into())), false),
        Field::new("util_5h", DataType::Float64, true),
        Field::new("util_7d", DataType::Float64, true),
        Field::new("burn_5h_cost_weighted", DataType::Float64, true),
        Field::new("burn_7d_cost_weighted", DataType::Float64, true),
        Field::new("input_5h", DataType::Int64, true),
        Field::new("cache_creation_5h", DataType::Int64, true),
        Field::new("cache_read_5h", DataType::Int64, true),
        Field::new("output_5h", DataType::Int64, true),
        Field::new("input_7d", DataType::Int64, true),
        Field::new("cache_creation_7d", DataType::Int64, true),
        Field::new("cache_read_7d", DataType::Int64, true),
        Field::new("output_7d", DataType::Int64, true),
        Field::new("subscription_type", DataType::Utf8, false),
        Field::new("rate_limit_tier", DataType::Utf8, false),
        Field::new("resets_5h_iso", DataType::Utf8, true),
        Field::new("resets_7d_iso", DataType::Utf8, true),
    ]));

    let sampled_at = TimestampMillisecondArray::from(
        samples.iter().map(|s| s.ts.timestamp_millis()).collect::<Vec<i64>>(),
    )
    .with_timezone("UTC");

    let null_f64 = || Arc::new(Float64Array::from(vec![None::<f64>; n])) as ArrayRef;
    let null_i64 = || Arc::new(Int64Array::from(vec![None::<i64>; n])) as ArrayRef;

    let columns: Vec<ArrayRef> = vec![
        Arc::new(sampled_at),
        Arc::new(Float64Array::from(samples.iter().map(|s| s.five_hour_util).collect::<Vec<Option<f64>>>())),
        Arc::new(Float64Array::from(samples.iter().map(|s| s.seven_day_util).collect::<Vec<Option<f64>>>())),
        null_f64(),
        null_f64(),
        // input, cache_creation, cache_read, output — _5h block then _7d block
        null_i64(), null_i64(), null_i64(), null_i64(),
        null_i64(), null_i64(), null_i64(), null_i64(),
        Arc::new(StringArray::from_iter_values(samples.iter().map(|s| s.subscription_type.clone()))),
        Arc::new(StringArray::from_iter_values(samples.iter().map(|s| s.rate_limit_tier.clone()))),
        Arc::new(StringArray::from(samples.iter().map(|s| s.five_hour_resets_at.map(|d| d.to_rfc3339())).collect::<Vec<Option<String>>>())),
        Arc::new(StringArray::from(samples.iter().map(|s| s.seven_day_resets_at.map(|d| d.to_rfc3339())).collect::<Vec<Option<String>>>())),
    ];

    let batch = RecordBatch::try_new(schema.clone(), columns)?;
    write_parquet(schema, &batch)
}

/// Mirrors caps.py `DerivedCaps`. Field order matches the Python dataclass.
/// `Option::None` serializes to JSON `null`; `serde_json` always emits the key.
#[derive(Debug, Serialize)]
struct CapsJson {
    max5x_5h: Option<f64>,
    max5x_weekly: Option<f64>,
    pro_5h: Option<f64>,
    pro_weekly: Option<f64>,
    sampled_at: Option<String>,
    sample_burn_5h: Option<f64>,
    sample_burn_7d: Option<f64>,
    sample_util_5h: Option<f64>,
    sample_util_7d: Option<f64>,
    subscription_type: Option<String>,
    resets_5h_iso: Option<String>,
    resets_7d_iso: Option<String>,
    rate_limit_tier: Option<String>,
}

/// Build caps.json bytes (pretty-printed, like the Python agent).
pub fn caps_json(snapshot: &AppSnapshot, creds: &Credentials) -> Result<Vec<u8>> {
    let (sampled_at, util_5h, util_7d, resets_5h, resets_7d) = match &snapshot.last_sample {
        Some((usage, at)) => (
            Some(at.to_rfc3339()),
            usage.five_hour.as_ref().map(|b| b.utilization),
            usage.seven_day.as_ref().map(|b| b.utilization),
            usage.five_hour.as_ref().and_then(|b| b.resets_at).map(|d| d.to_rfc3339()),
            usage.seven_day.as_ref().and_then(|b| b.resets_at).map(|d| d.to_rfc3339()),
        ),
        None => (None, None, None, None, None),
    };

    let caps = CapsJson {
        max5x_5h: None,
        max5x_weekly: None,
        pro_5h: None,
        pro_weekly: None,
        sampled_at,
        sample_burn_5h: None,
        sample_burn_7d: None,
        sample_util_5h: util_5h,
        sample_util_7d: util_7d,
        subscription_type: Some(creds.subscription_type.clone()),
        resets_5h_iso: resets_5h,
        resets_7d_iso: resets_7d,
        rate_limit_tier: Some(creds.rate_limit_tier.clone()),
    };

    Ok(serde_json::to_vec_pretty(&caps)?)
}

/// Stream a single RecordBatch into an in-memory parquet buffer.
fn write_parquet(schema: Arc<Schema>, batch: &RecordBatch) -> Result<Vec<u8>> {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut writer = ArrowWriter::try_new(&mut buf, schema, None)?;
        writer.write(batch)?;
        writer.close()?;
    }
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::parser::Turn;
    use chrono::TimeZone;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use std::io::Write;
    use std::path::PathBuf;

    fn sample_turn() -> Turn {
        Turn {
            ts: chrono::Utc.with_ymd_and_hms(2026, 5, 23, 10, 0, 0).unwrap(),
            session_id: "sess-1".into(),
            subagent_id: None,
            is_subagent: false,
            project_cwd: "C:/proj".into(),
            model: "claude-opus-4-7".into(),
            version: "1.0".into(),
            input_tokens: 100,
            output_tokens: 400,
            cache_creation_input_tokens: 200,
            cache_read_input_tokens: 300,
            source_file: PathBuf::from("a.jsonl"),
            is_rate_limit_error: false,
        }
    }

    /// Write bytes to a temp file and read the parquet back into a RecordBatch.
    /// When the file has 0 rows (no row groups) the reader yields nothing;
    /// in that case we build an empty batch from the file metadata's schema.
    fn read_back(bytes: &[u8]) -> arrow::record_batch::RecordBatch {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(bytes).unwrap();
        let file = f.reopen().unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
        // Capture the arrow schema before the builder is consumed.
        let arrow_schema = builder.schema().clone();
        let mut reader = builder.build().unwrap();
        match reader.next() {
            Some(batch) => batch.unwrap(),
            // 0-row file: synthesise an empty batch so column-count checks work.
            None => arrow::record_batch::RecordBatch::new_empty(arrow_schema),
        }
    }

    #[test]
    fn cache_parquet_roundtrips_schema_and_values() {
        let bytes = cache_parquet(&[sample_turn()]).unwrap();
        let batch = read_back(&bytes);

        let schema = batch.schema();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, vec![
            "timestamp", "session_id", "subagent_id", "is_subagent", "project_cwd",
            "model", "version", "input_tokens", "output_tokens",
            "cache_creation_input_tokens", "cache_read_input_tokens",
            "source_file", "is_rate_limit_error",
        ]);
        assert_eq!(batch.num_rows(), 1);

        use arrow::array::{Int64Array, StringArray};
        let out = batch.column(8).as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(out.value(0), 400);
        let sess = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(sess.value(0), "sess-1");
    }

    #[test]
    fn cache_parquet_preserves_some_subagent_id() {
        use arrow::array::{Array, StringArray};
        let mut t = sample_turn();
        t.subagent_id = Some("agent-abc".into());
        let bytes = cache_parquet(&[t]).unwrap();
        let batch = read_back(&bytes);
        let col = batch.column(2).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(col.value(0), "agent-abc");
        assert!(!col.is_null(0));
    }

    #[test]
    fn cache_parquet_handles_empty() {
        let bytes = cache_parquet(&[]).unwrap();
        let batch = read_back(&bytes);
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.schema().fields().len(), 13);
    }

    fn sample_calib() -> CalibrationSample {
        CalibrationSample {
            schema_version: 1,
            ts: chrono::Utc.with_ymd_and_hms(2026, 5, 23, 9, 30, 0).unwrap(),
            five_hour_util: Some(0.42),
            five_hour_resets_at: Some(chrono::Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap()),
            seven_day_util: Some(0.10),
            seven_day_resets_at: None,
            subscription_type: "pro".into(),
            rate_limit_tier: "default".into(),
        }
    }

    #[test]
    fn calib_log_parquet_handles_empty() {
        let bytes = calibration_log_parquet(&[]).unwrap();
        let batch = read_back(&bytes);
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.schema().fields().len(), 17);
    }

    use crate::api::credentials::Credentials;
    use crate::api::usage::{UsageBucket, UsageSnapshot};
    use crate::shared::snapshot::AppSnapshot;

    #[test]
    fn caps_json_populates_from_snapshot_and_nulls_the_rest() {
        let usage = UsageSnapshot {
            five_hour: Some(UsageBucket {
                utilization: 0.42,
                resets_at: Some(chrono::Utc.with_ymd_and_hms(2026, 5, 23, 12, 0, 0).unwrap()),
            }),
            seven_day: Some(UsageBucket { utilization: 0.1, resets_at: None }),
        };
        let sampled = chrono::Utc.with_ymd_and_hms(2026, 5, 23, 9, 30, 0).unwrap();
        let snapshot = AppSnapshot {
            last_sample: Some((usage, sampled)),
            ..Default::default()
        };
        let creds = Credentials {
            access_token: "t".into(),
            subscription_type: "pro".into(),
            rate_limit_tier: "default".into(),
        };

        let bytes = caps_json(&snapshot, &creds).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(v["sample_util_5h"], 0.42);
        assert_eq!(v["sample_util_7d"], 0.1);
        assert_eq!(v["subscription_type"], "pro");
        assert_eq!(v["rate_limit_tier"], "default");
        assert_eq!(v["sampled_at"], sampled.to_rfc3339());
        assert_eq!(v["resets_5h_iso"], "2026-05-23T12:00:00+00:00");
        assert!(v["resets_7d_iso"].is_null());
        assert!(v["max5x_5h"].is_null());
        assert!(v["pro_weekly"].is_null());
        assert!(v["sample_burn_5h"].is_null());
        assert_eq!(v.as_object().unwrap().len(), 13);
    }

    #[test]
    fn caps_json_handles_no_sample() {
        let creds = Credentials { access_token: "t".into(), subscription_type: "pro".into(), rate_limit_tier: "default".into() };
        let bytes = caps_json(&AppSnapshot::default(), &creds).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v["sampled_at"].is_null());
        assert!(v["sample_util_5h"].is_null());
        assert_eq!(v["subscription_type"], "pro");
    }

    #[test]
    fn calib_log_parquet_schema_values_and_nulls() {
        let bytes = calibration_log_parquet(&[sample_calib()]).unwrap();
        let batch = read_back(&bytes);

        let schema = batch.schema();
        let names: Vec<&str> = schema.fields().iter().map(|f| f.name().as_str()).collect();
        assert_eq!(names, vec![
            "sampled_at", "util_5h", "util_7d",
            "burn_5h_cost_weighted", "burn_7d_cost_weighted",
            "input_5h", "cache_creation_5h", "cache_read_5h", "output_5h",
            "input_7d", "cache_creation_7d", "cache_read_7d", "output_7d",
            "subscription_type", "rate_limit_tier", "resets_5h_iso", "resets_7d_iso",
        ]);
        assert_eq!(batch.num_rows(), 1);

        use arrow::datatypes::{DataType, TimeUnit};
        assert_eq!(
            batch.schema().field(0).data_type(),
            &DataType::Timestamp(TimeUnit::Millisecond, Some("UTC".into()))
        );

        use arrow::array::{Array, Float64Array, StringArray};
        let u5 = batch.column(1).as_any().downcast_ref::<Float64Array>().unwrap();
        assert!((u5.value(0) - 0.42).abs() < 1e-9);

        let burn5 = batch.column(3).as_any().downcast_ref::<Float64Array>().unwrap();
        assert!(burn5.is_null(0));

        let r7 = batch.column(16).as_any().downcast_ref::<StringArray>().unwrap();
        assert!(r7.is_null(0));

        let sub = batch.column(13).as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(sub.value(0), "pro");
    }
}

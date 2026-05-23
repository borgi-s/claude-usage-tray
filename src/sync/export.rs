//! Serialize in-memory/on-disk state into parquet + JSON byte buffers that the
//! polars cloud viewer reads. Schemas mirror the Python project exactly.

use crate::data::parser::Turn;
use anyhow::Result;
use arrow::array::{ArrayRef, BooleanArray, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
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
    fn cache_parquet_handles_empty() {
        let bytes = cache_parquet(&[]).unwrap();
        let batch = read_back(&bytes);
        assert_eq!(batch.num_rows(), 0);
        assert_eq!(batch.schema().fields().len(), 13);
    }
}

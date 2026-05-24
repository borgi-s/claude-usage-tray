//! Per-session aggregation for the dashboard's sessions table. Pure functions
//! ported from the Python `metrics.session_summaries`. No egui here.

use crate::data::parser::Turn;

/// Context tokens fed into the model for one turn:
/// input + cache_creation + cache_read. Output is excluded (it's the response,
/// not the context). Mirrors the Python `prompt_tokens` derivation.
pub fn prompt_tokens(t: &Turn) -> u64 {
    t.input_tokens + t.cache_creation_input_tokens + t.cache_read_input_tokens
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::path::PathBuf;

    fn turn(input: u64, cc: u64, cr: u64, output: u64) -> Turn {
        Turn {
            ts: Utc::now(),
            session_id: String::new(),
            subagent_id: None,
            is_subagent: false,
            project_cwd: String::new(),
            model: String::new(),
            version: String::new(),
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: cc,
            cache_read_input_tokens: cr,
            source_file: PathBuf::new(),
            is_rate_limit_error: false,
        }
    }

    #[test]
    fn prompt_tokens_sums_inputs_excludes_output() {
        // 100 + 200 + 300 = 600; output 999 ignored.
        let t = turn(100, 200, 300, 999);
        assert_eq!(prompt_tokens(&t), 600);
    }
}

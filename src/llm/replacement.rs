//! Purpose: this file must parse the bounded marked-region replacement fallback.
//! Owns: the exact JSON response envelope and replacement output size limit.
//! Must not: choose an edit range, mutate buffers, accept prose, or perform I/O.
//! Invariants: only one named string field is valid; oversized output fails closed.
//! Phase: 6 (LLM, Powerful but Caged).

use serde::Deserialize;

pub const MAX_REPLACEMENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplacementError {
    Malformed,
    TooLarge { bytes: usize },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplacementEnvelope {
    catomic_replacement: String,
}

pub fn parse(text: &str) -> Result<String, ReplacementError> {
    let envelope: ReplacementEnvelope =
        serde_json::from_str(text).map_err(|_| ReplacementError::Malformed)?;
    let bytes = envelope.catomic_replacement.len();
    if bytes > MAX_REPLACEMENT_BYTES {
        return Err(ReplacementError::TooLarge { bytes });
    }
    Ok(envelope.catomic_replacement)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_replacement_envelope_and_preserves_text() {
        assert_eq!(
            parse(r#"{"catomic_replacement":"one\n猫\n"}"#),
            Ok("one\n猫\n".to_string())
        );
        assert_eq!(parse(r#"{"catomic_replacement":""}"#), Ok(String::new()));
    }

    #[test]
    fn rejects_prose_wrong_types_extra_fields_and_duplicate_fields() {
        for invalid in [
            "replacement text",
            r#"{"replacement":"text"}"#,
            r#"{"catomic_replacement":7}"#,
            r#"{"catomic_replacement":"text","note":"prose"}"#,
            r#"{"catomic_replacement":"one","catomic_replacement":"two"}"#,
        ] {
            assert_eq!(parse(invalid), Err(ReplacementError::Malformed));
        }
    }

    #[test]
    fn rejects_replacement_beyond_the_hard_limit() {
        let json = serde_json::json!({
            "catomic_replacement": "x".repeat(MAX_REPLACEMENT_BYTES + 1)
        })
        .to_string();
        assert_eq!(
            parse(&json),
            Err(ReplacementError::TooLarge {
                bytes: MAX_REPLACEMENT_BYTES + 1
            })
        );
    }
}

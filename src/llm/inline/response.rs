//! Purpose: parse strict request-local multi-block replacement envelopes.
//! Owns: identifier uniqueness/completeness and bounded replacement response validation.
//! Must not: discover ranges, mutate buffers, accept prose, patches, or unknown JSON fields.
//! Invariants: every expected block appears exactly once and no replacement exceeds hard limits.
//! Phase: issue #65 one-key inline clanker workflow.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::llm::replacement::MAX_REPLACEMENT_BYTES;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseError {
    Malformed,
    MissingBlock { id: usize },
    UnexpectedBlock { id: usize },
    DuplicateBlock { id: usize },
    TooLarge { id: usize, bytes: usize },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    catomic_replacements: Vec<BlockReplacement>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockReplacement {
    block: usize,
    replacement: String,
}

pub fn parse_combined_replacements(
    text: &str,
    expected_ids: &[usize],
) -> Result<BTreeMap<usize, String>, ResponseError> {
    let envelope: Envelope = serde_json::from_str(text).map_err(|_| ResponseError::Malformed)?;
    let mut replacements = BTreeMap::new();
    for block in envelope.catomic_replacements {
        if !expected_ids.contains(&block.block) {
            return Err(ResponseError::UnexpectedBlock { id: block.block });
        }
        if block.replacement.len() > MAX_REPLACEMENT_BYTES {
            return Err(ResponseError::TooLarge {
                id: block.block,
                bytes: block.replacement.len(),
            });
        }
        if replacements
            .insert(block.block, block.replacement)
            .is_some()
        {
            return Err(ResponseError::DuplicateBlock { id: block.block });
        }
    }
    for &id in expected_ids {
        if !replacements.contains_key(&id) {
            return Err(ResponseError::MissingBlock { id });
        }
    }
    Ok(replacements)
}

//! Ignored retained-storage evidence for bounded PieceTable history.
//!
//! Owns: a deterministic large paste/delete-cycle sample that reports elapsed
//! time and retained history/add-buffer sizes after default-policy pruning.
//! Must not: run by default, enforce timing budgets, or add dependencies.

use crate::buffer::undo::{DEFAULT_UNDO_MAX_BYTES, DEFAULT_UNDO_MAX_TRANSACTIONS};
use crate::buffer::{Buffer, Cursor, PieceTable};

use super::helpers::{measure_sample, print_perf_sample};

#[test]
#[ignore = "manual retained-memory sample; processes 48 MiB of inserted text"]
fn manual_undo_retention_large_paste_delete_cycles() {
    const CHUNK_BYTES: usize = 1024 * 1024;
    const CYCLES: usize = 48;

    let payload = "x".repeat(CHUNK_BYTES);
    let mut buffer = PieceTable::new();
    let (_, sample) = measure_sample(
        "undo retention 48x1mib paste-delete",
        Some((CHUNK_BYTES * CYCLES) as u64),
        || {
            for _ in 0..CYCLES {
                buffer
                    .replace_range(
                        Cursor { row: 0, col: 0 },
                        Cursor { row: 0, col: 0 },
                        &payload,
                    )
                    .unwrap();
                buffer
                    .replace_range(
                        Cursor { row: 0, col: 0 },
                        Cursor {
                            row: 0,
                            col: CHUNK_BYTES,
                        },
                        "",
                    )
                    .unwrap();
            }
        },
    );
    print_perf_sample(&sample);

    let transactions = buffer.retained_history_transactions_for_test();
    let history_bytes = buffer.retained_history_bytes_for_test();
    let (add_len, add_capacity) = buffer.add_storage_for_test();
    eprintln!(
        "PERF retained: transactions={transactions} history_bytes={history_bytes} \
         add_len={add_len} add_capacity={add_capacity} logical_bytes={}",
        buffer.logical_byte_len().unwrap_or_default()
    );

    assert_eq!(buffer.logical_byte_len(), Some(0));
    assert!(transactions <= DEFAULT_UNDO_MAX_TRANSACTIONS);
    assert!(history_bytes <= DEFAULT_UNDO_MAX_BYTES);
    assert!(
        add_len < CHUNK_BYTES * CYCLES,
        "thresholded compaction must reclaim unreachable paste storage"
    );
    assert!(
        add_capacity < CHUNK_BYTES * CYCLES,
        "thresholded compaction must also reduce retained String capacity"
    );
}

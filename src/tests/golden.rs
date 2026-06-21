//! Golden tests: input file + sequence of operations → exact file output.
//!
//! Non-negotiable for buffer correctness.
//! Especially important around undo, save, external edit conflict, patch apply.

#[cfg(test)]
mod tests {
    // TODO: load fixture, drive buffer or app, compare resulting file bytes.
}

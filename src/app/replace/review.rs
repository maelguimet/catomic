//! Candidate review and bounded bulk replacement. Each scan starts at its next
//! unhandled byte. Wrapping visits only the prefix before the original cursor,
//! whose boundary moves with accepted prefix edits; inserted text is never revisited.

use std::io;

use crate::app::App;
use crate::buffer::Cursor;
use crate::config::actions::{display_chord, Action};
use crate::editor::search::{ForwardMatch, ForwardSearchTask, SearchMatch};
use crate::terminal::TerminalOutput;

const SCAN_BYTES: usize = 32 * 1024;
const MAX_BATCH_MATCHES: usize = 128;
const MAX_BATCH_EDIT_BYTES: usize = 64 * 1024;

pub(super) struct Review {
    find: String,
    replacement: String,
    scan: ForwardSearchTask,
    candidate: Option<ForwardMatch>,
    next_byte: usize,
    boundary: usize,
    first_accepted: Option<usize>,
    wrapped: bool,
    all: bool,
    replaced: usize,
    skipped: usize,
    buffer_id: u64,
    generation: u64,
    revision: u64,
}

impl Review {
    fn current(&self, app: &App) -> bool {
        self.buffer_id == app.file.buffer_id
            && self.generation == app.file.content_generation
            && self.revision == app.buffer.content_revision()
            && app.buffer.page_info().is_none()
    }

    fn resume(&mut self) {
        self.candidate = None;
        let stop_before = self.wrapped.then(|| {
            // A wrapped candidate may cross the untouched original cursor, but
            // must end before the first accepted tail replacement begins.
            self.first_accepted.map_or(self.boundary, |first| {
                self.boundary.min(first.saturating_sub(self.find.len() - 1))
            })
        });
        self.scan = ForwardSearchTask::new(&self.find, self.next_byte, stop_before);
    }
}

pub(super) fn start(
    app: &mut App,
    out: &mut dyn TerminalOutput,
    find: &str,
    replacement: &str,
    all: bool,
) -> io::Result<()> {
    let Some(source) = app.buffer.piece_table_search() else {
        app.message_error("Replace requires a buffer with bounded editable search.");
        return app.render(out);
    };
    let origin = if all {
        0
    } else {
        match source.byte_offset_for_cursor(app.buffer.cursor()) {
            Ok(origin) => origin,
            Err(error) => {
                app.message_error(format!("Replacement could not start: {error}"));
                return app.render(out);
            }
        }
    };
    app.replace.review = Some(Review {
        find: find.into(),
        replacement: replacement.into(),
        scan: ForwardSearchTask::new(find, origin, None),
        candidate: None,
        next_byte: origin,
        boundary: origin,
        first_accepted: None,
        wrapped: false,
        all,
        replaced: 0,
        skipped: 0,
        buffer_id: app.file.buffer_id,
        generation: app.file.content_generation,
        revision: app.buffer.content_revision(),
    });
    update_message(app);
    poll(app, out)
}

pub(crate) fn is_running(app: &App) -> bool {
    !app.pending_quit_confirm
        && app
            .replace
            .review
            .as_ref()
            .is_some_and(|review| review.candidate.is_none())
}

pub(crate) fn active_match(app: &App) -> Option<SearchMatch> {
    app.replace
        .review
        .as_ref()
        .filter(|review| review.current(app))?
        .candidate
        .map(|candidate| candidate.found)
}

pub(super) fn dispatch_action(
    app: &mut App,
    out: &mut dyn TerminalOutput,
    action: Action,
) -> io::Result<bool> {
    let Some(mut review) = app.replace.review.take() else {
        return Ok(false);
    };
    if !review.current(app) {
        finish(app, out, &review, "stopped because the source changed")?;
        return Ok(true);
    }
    match action {
        Action::ReplaceCancel => {
            finish(app, out, &review, "cancelled")?;
            return Ok(true);
        }
        Action::ReplaceAccept => {
            if let Some(candidate) = review.candidate {
                if let Err(error) = apply(app, &mut review, &[candidate]) {
                    fail(app, out, &review, error)?;
                    return Ok(true);
                }
            }
        }
        Action::ReplaceSkip => {
            if let Some(candidate) = review.candidate {
                // Search is scalar aligned and overlapping candidates remain
                // available after Skip. Accept instead resumes after the insertion.
                review.next_byte =
                    candidate.start_byte + review.find.chars().next().map_or(1, char::len_utf8);
                review.skipped += 1;
                review.resume();
            }
        }
        Action::ReplaceRemaining => {
            review.all = true;
            if let Some(candidate) = review.candidate {
                review.next_byte = candidate.start_byte;
            }
            review.resume();
        }
        _ => {
            app.replace.review = Some(review);
            update_message(app);
            app.render(out)?;
            return Ok(true);
        }
    }
    app.replace.review = Some(review);
    update_message(app);
    poll(app, out)?;
    Ok(true)
}

pub(crate) fn poll(app: &mut App, out: &mut dyn TerminalOutput) -> io::Result<()> {
    let Some(mut review) = app.replace.review.take() else {
        return Ok(());
    };
    if !review.current(app) {
        return finish(app, out, &review, "stopped because the source changed");
    }
    if app.pending_quit_confirm || review.candidate.is_some() {
        app.replace.review = Some(review);
        return Ok(());
    }
    // At most two bounded scans: the second is permitted only when wrapping.
    for _ in 0..2 {
        let count = if review.all {
            MAX_BATCH_MATCHES
                .min(MAX_BATCH_EDIT_BYTES / (review.find.len() + review.replacement.len()).max(1))
                .max(1)
        } else {
            1
        };
        let result = match review.scan.poll(&*app.buffer, SCAN_BYTES, count) {
            Ok(result) => result,
            Err(error) => return fail(app, out, &review, error),
        };
        match result {
            None => break,
            Some(matches) if matches.is_empty() => {
                if review.wrapped || review.boundary == 0 {
                    return finish(app, out, &review, "complete");
                }
                review.wrapped = true;
                review.next_byte = 0;
                review.resume();
            }
            Some(mut matches) => {
                if review.all {
                    let mut next = review.next_byte;
                    matches.retain(|found| {
                        if found.start_byte < next {
                            return false;
                        }
                        next = found.end_byte;
                        true
                    });
                    if let Err(error) = apply(app, &mut review, &matches) {
                        return fail(app, out, &review, error);
                    }
                    if (review.wrapped && review.next_byte >= review.boundary)
                        || (review.boundary == 0
                            && app.buffer.logical_byte_len() == Some(review.next_byte))
                    {
                        return finish(app, out, &review, "complete");
                    }
                } else if let Some(candidate) = matches.first().copied() {
                    review.candidate = Some(candidate);
                    app.buffer.set_cursor(candidate.found.start);
                    app.reveal_cursor();
                }
                break;
            }
        }
    }
    app.replace.review = Some(review);
    update_message(app);
    app.render(out)
}

fn apply(app: &mut App, review: &mut Review, matches: &[ForwardMatch]) -> io::Result<()> {
    let Some(last) = matches.last() else {
        return Ok(());
    };
    let ranges: Vec<_> = matches
        .iter()
        .rev()
        .map(|found| {
            (
                found.found.start,
                Cursor {
                    row: found.found.start.row,
                    col: found.found.end_col,
                },
            )
        })
        .collect();
    let delta = review.replacement.len() as isize - review.find.len() as isize;
    let prefix_edits = matches
        .iter()
        .filter(|found| found.start_byte < review.boundary)
        .count();
    if review.find != review.replacement {
        let revision = app.buffer.content_revision();
        let result = app.buffer.replace_ranges(&ranges, &review.replacement);
        if app.buffer.content_revision() != revision {
            crate::app::input::record_content_edit(app, None)?;
            crate::app::completion::cancel(app);
        }
        result?;
    }
    if !review.wrapped && review.first_accepted.is_none() {
        review.first_accepted = matches.first().map(|found| found.start_byte);
    } else if review.wrapped {
        review.first_accepted = review
            .first_accepted
            .map(|first| first.saturating_add_signed(delta * prefix_edits as isize));
    }
    review.boundary = review
        .boundary
        .saturating_add_signed(delta * prefix_edits as isize);
    review.next_byte = last
        .end_byte
        .saturating_add_signed(delta * matches.len() as isize);
    review.replaced += matches.len();
    review.generation = app.file.content_generation;
    review.revision = app.buffer.content_revision();
    let source = app
        .buffer
        .piece_table_search()
        .ok_or_else(|| io::Error::other("replacement source unavailable"))?;
    app.buffer
        .set_cursor(source.cursor_for_byte_offset(review.next_byte)?);
    app.reveal_cursor();
    review.resume();
    Ok(())
}

fn finish(
    app: &mut App,
    out: &mut dyn TerminalOutput,
    review: &Review,
    state: &str,
) -> io::Result<()> {
    app.message_info(format!(
        "Replacement {state}: {} replaced, {} skipped.",
        review.replaced, review.skipped
    ));
    app.render(out)
}

fn fail(
    app: &mut App,
    out: &mut dyn TerminalOutput,
    review: &Review,
    error: io::Error,
) -> io::Result<()> {
    app.message_error(format!(
        "Replacement stopped: {error}; {} replaced, {} skipped.",
        review.replaced, review.skipped
    ));
    app.render(out)
}

fn chord(app: &App, action: Action) -> String {
    app.keybindings
        .keyboard_chords(action)
        .into_iter()
        .min_by_key(String::len)
        .map(|key| display_chord(&key))
        .unwrap_or_else(|| "unbound".into())
}

fn update_message(app: &mut App) {
    let Some(review) = app.replace.review.as_ref() else {
        return;
    };
    let cancel = chord(app, Action::ReplaceCancel);
    let message = if let Some(candidate) = review.candidate {
        format!(
            "Replace candidate {}:{}; {} replace, {} skip, {} all, {cancel} cancel",
            candidate.found.start.row + 1,
            candidate.found.start.col + 1,
            chord(app, Action::ReplaceAccept),
            chord(app, Action::ReplaceSkip),
            chord(app, Action::ReplaceRemaining)
        )
    } else if review.all {
        format!(
            "Replacing remaining: {} replaced; {cancel} cancels.",
            review.replaced
        )
    } else {
        format!("Finding replacement candidate; {cancel} cancels.")
    };
    app.message_info(message);
}

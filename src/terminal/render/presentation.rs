//! Purpose: retain published terminal rows and emit only damaged rows.
//! Owns: bounded row plans, exact row fingerprints, candidate bytes, and transactional publish.
//! Must not: mutate buffers, retain borrowed buffer data, publish before flush, or own setup.
//! Invariants: failed composition/transport invalidates all rows; a new plan is published only
//!   after its complete synchronized frame is written and flushed.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};

use crate::buffer::{Buffer, Cursor};

use super::{frame, style, wrapped, RenderOptions, RenderViewport};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PresentationMetrics {
    pub(crate) rows_composed: usize,
    pub(crate) rows_emitted: usize,
    pub(crate) output_bytes: usize,
}

#[derive(Default)]
struct PublishedRow {
    fingerprint: u64,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct CandidateRow {
    fingerprint: u64,
    bytes: Option<Vec<u8>>,
    emit: bool,
}

enum RetainedPlan {
    Unwrapped(frame::RowPlan),
    Wrapped(wrapped::RowPlan),
}

impl RetainedPlan {
    fn content_height(&self) -> usize {
        match self {
            Self::Unwrapped(plan) => plan.content_height,
            Self::Wrapped(plan) => plan.content_height,
        }
    }

    fn cursor_position(&self, cursor: Cursor) -> Option<(usize, usize)> {
        match self {
            Self::Unwrapped(plan) => plan.cursor_position(cursor),
            Self::Wrapped(plan) => plan.cursor_position(cursor),
        }
    }

    fn row_fingerprint(&self, row: usize, options: RenderOptions<'_>) -> u64 {
        match self {
            Self::Unwrapped(plan) => frame::row_fingerprint(plan, row, options),
            Self::Wrapped(plan) => wrapped::row_fingerprint(plan, row, options),
        }
    }

    fn compose_row(
        &self,
        out: &mut Vec<u8>,
        buffer: &dyn Buffer,
        row: usize,
        options: RenderOptions<'_>,
        boundaries: &mut Vec<usize>,
    ) -> io::Result<()> {
        match self {
            Self::Unwrapped(plan) => {
                frame::compose_row(out, buffer, plan, row, options, boundaries)
            }
            Self::Wrapped(plan) => {
                wrapped::compose_row(out, buffer, plan, row, options, boundaries)
            }
        }
    }
}

#[derive(Clone, Copy)]
struct ComponentChanges {
    header: bool,
    bottom: bool,
    overlay: bool,
}

#[derive(Default)]
pub(crate) struct PresentationState {
    rows: Vec<PublishedRow>,
    candidates: Vec<CandidateRow>,
    row_pool: Vec<Vec<u8>>,
    plan: Option<RetainedPlan>,
    plan_input: u64,
    layout_input: u64,
    frame: Vec<u8>,
    header: Vec<u8>,
    header_candidate: Vec<u8>,
    bottom: Vec<u8>,
    bottom_candidate: Vec<u8>,
    overlay: Vec<u8>,
    overlay_candidate: Vec<u8>,
    cursor: Vec<u8>,
    boundaries: Vec<usize>,
    header_input: u64,
    bottom_input: u64,
    overlay_input: u64,
    invalidated: bool,
    had_emoji_picker: bool,
    metrics: PresentationMetrics,
}

impl PresentationState {
    pub(crate) fn invalidate(&mut self) {
        self.invalidated = true;
    }

    #[cfg(test)]
    pub(crate) fn metrics(&self) -> PresentationMetrics {
        self.metrics
    }

    #[cfg(test)]
    pub(crate) fn published_row_count(&self) -> usize {
        self.rows.len()
    }

    pub(crate) fn render<W: Write + ?Sized>(
        &mut self,
        out: &mut W,
        buffer: &dyn Buffer,
        viewport: RenderViewport,
        message: Option<&str>,
        options: RenderOptions<'_>,
    ) -> io::Result<()> {
        super::validate_frame_size(viewport)?;
        self.recycle_candidates();
        self.metrics = PresentationMetrics::default();

        let layout_input = layout_fingerprint(buffer, viewport, options);
        let plan_input = plan_fingerprint(layout_input, options);
        let mut published_plan = self.plan.take();
        let replace_plan = published_plan.is_none() || self.plan_input != plan_input;
        let layout_changed = published_plan.is_none() || self.layout_input != layout_input;
        let mut candidate_plan = None;
        let plan = if replace_plan {
            candidate_plan = Some(match build_plan(buffer, viewport, options) {
                Ok(plan) => plan,
                Err(error) => {
                    self.invalidated = true;
                    self.plan = published_plan;
                    return Err(error);
                }
            });
            candidate_plan.as_ref().expect("candidate plan was built")
        } else {
            published_plan
                .as_ref()
                .expect("matching retained plan exists")
        };

        let emoji_picker_active = options.emoji_picker.is_some();
        let force_all_rows =
            self.invalidated || layout_changed || emoji_picker_active || self.had_emoji_picker;
        let cursor_position = plan.cursor_position(buffer.cursor());
        if let Err(error) = self.compose_candidate_rows(buffer, plan, options, force_all_rows) {
            self.invalidated = true;
            self.recycle_candidates();
            self.plan = published_plan;
            return Err(error);
        }

        let component_inputs = component_fingerprints(viewport, message, options, cursor_position);
        let changes = ComponentChanges {
            header: force_all_rows || self.header_input != component_inputs.0,
            bottom: force_all_rows || self.bottom_input != component_inputs.1,
            overlay: force_all_rows
                || self.overlay_input != component_inputs.2
                || emoji_picker_active,
        };
        if let Err(error) = self.compose_frame(viewport, message, options, cursor_position, changes)
        {
            self.invalidated = true;
            self.recycle_candidates();
            self.plan = published_plan;
            return Err(error);
        }
        self.metrics.output_bytes = self.frame.len();
        if let Err(error) = out.write_all(&self.frame).and_then(|()| out.flush()) {
            self.invalidated = true;
            self.plan = published_plan;
            return Err(error);
        }

        let content_height = plan.content_height();
        self.publish_candidates(content_height);
        if changes.header {
            Self::publish_component(&mut self.header, &mut self.header_candidate);
        }
        if changes.bottom {
            Self::publish_component(&mut self.bottom, &mut self.bottom_candidate);
        }
        if changes.overlay {
            Self::publish_component(&mut self.overlay, &mut self.overlay_candidate);
        }
        if let Some(candidate_plan) = candidate_plan {
            self.plan = Some(candidate_plan);
            self.plan_input = plan_input;
            self.layout_input = layout_input;
        } else {
            self.plan = published_plan.take();
        }
        self.header_input = component_inputs.0;
        self.bottom_input = component_inputs.1;
        self.overlay_input = component_inputs.2;
        self.invalidated = false;
        self.had_emoji_picker = emoji_picker_active;
        Ok(())
    }

    fn compose_candidate_rows(
        &mut self,
        buffer: &dyn Buffer,
        plan: &RetainedPlan,
        options: RenderOptions<'_>,
        force: bool,
    ) -> io::Result<()> {
        for row in 0..plan.content_height() {
            let fingerprint = plan.row_fingerprint(row, options);
            let published = self.rows.get(row);
            if !force && published.is_some_and(|cached| cached.fingerprint == fingerprint) {
                self.candidates.push(CandidateRow {
                    fingerprint,
                    bytes: None,
                    emit: false,
                });
                continue;
            }

            let mut bytes = self.row_pool.pop().unwrap_or_default();
            bytes.clear();
            if let Err(error) =
                plan.compose_row(&mut bytes, buffer, row, options, &mut self.boundaries)
            {
                self.row_pool.push(bytes);
                return Err(error);
            }
            self.metrics.rows_composed = self.metrics.rows_composed.saturating_add(1);
            let emit = force || published.is_none_or(|cached| cached.bytes != bytes);
            if emit {
                self.metrics.rows_emitted = self.metrics.rows_emitted.saturating_add(1);
            }
            self.candidates.push(CandidateRow {
                fingerprint,
                bytes: Some(bytes),
                emit,
            });
        }
        Ok(())
    }

    fn compose_frame(
        &mut self,
        viewport: RenderViewport,
        message: Option<&str>,
        options: RenderOptions<'_>,
        cursor_position: Option<(usize, usize)>,
        changes: ComponentChanges,
    ) -> io::Result<()> {
        if changes.header {
            self.header_candidate.clear();
            super::super::title::write(&mut self.header_candidate, options.window_title)?;
            style::write_cursor_color(&mut self.header_candidate, options.theme)?;
        }
        if changes.bottom {
            self.bottom_candidate.clear();
            super::write_bottom_rows(&mut self.bottom_candidate, viewport, message, options)?;
        }
        if changes.overlay {
            self.overlay_candidate.clear();
            super::emoji_picker::write(
                &mut self.overlay_candidate,
                cursor_position,
                super::content_height(viewport.height, options.action_bar),
                viewport.width,
                options.emoji_picker,
                options.theme,
            )?;
        }
        self.cursor.clear();
        super::write_terminal_cursor(&mut self.cursor, cursor_position, options.cursor_shape)?;

        self.frame.clear();
        super::begin_frame(&mut self.frame)?;
        if changes.header {
            self.frame.extend_from_slice(&self.header_candidate);
        }
        for candidate in &self.candidates {
            if candidate.emit {
                if let Some(bytes) = candidate.bytes.as_deref() {
                    self.frame.extend_from_slice(bytes);
                }
            }
        }
        if changes.bottom {
            self.frame.extend_from_slice(&self.bottom_candidate);
        }
        if changes.overlay {
            self.frame.extend_from_slice(&self.overlay_candidate);
        }
        self.frame.extend_from_slice(&self.cursor);
        super::end_frame(&mut self.frame)
    }

    fn publish_candidates(&mut self, content_height: usize) {
        while self.rows.len() > content_height {
            if let Some(row) = self.rows.pop() {
                self.row_pool.push(row.bytes);
            }
        }
        self.rows.resize_with(content_height, PublishedRow::default);
        for (row, candidate) in self.candidates.iter_mut().enumerate() {
            let published = &mut self.rows[row];
            published.fingerprint = candidate.fingerprint;
            let Some(bytes) = candidate.bytes.take() else {
                continue;
            };
            let old = std::mem::replace(&mut published.bytes, bytes);
            self.row_pool.push(old);
        }
    }

    fn publish_component(published: &mut Vec<u8>, candidate: &mut Vec<u8>) {
        std::mem::swap(published, candidate);
        candidate.clear();
    }

    fn recycle_candidates(&mut self) {
        for candidate in self.candidates.drain(..) {
            if let Some(bytes) = candidate.bytes {
                self.row_pool.push(bytes);
            }
        }
    }
}

fn build_plan(
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    options: RenderOptions<'_>,
) -> io::Result<RetainedPlan> {
    if options.soft_wrap {
        wrapped::plan_buffer(buffer, viewport, options).map(RetainedPlan::Wrapped)
    } else {
        frame::plan_buffer(buffer, viewport, options).map(RetainedPlan::Unwrapped)
    }
}

fn layout_fingerprint(
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    options: RenderOptions<'_>,
) -> u64 {
    fingerprint(|hash| {
        let line_gutter = if options.line_numbers {
            super::line_number_gutter(buffer.line_count())
        } else {
            0
        }
        .min(viewport.width);
        let external_gutter = super::change_gutter_width(
            options
                .external_changes
                .is_some_and(|changes| !changes.markers.is_empty()),
        )
        .min(viewport.width.saturating_sub(line_gutter));
        options.document_id.hash(hash);
        viewport.hash(hash);
        line_gutter.hash(hash);
        external_gutter.hash(hash);
        options.soft_wrap.hash(hash);
        options.action_bar.is_some().hash(hash);
    })
}

fn plan_fingerprint(layout: u64, options: RenderOptions<'_>) -> u64 {
    fingerprint(|hash| {
        layout.hash(hash);
        options.document_revision.hash(hash);
    })
}

fn component_fingerprints(
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions<'_>,
    cursor: Option<(usize, usize)>,
) -> (u64, u64, u64) {
    let header = fingerprint(|hash| {
        options.window_title.hash(hash);
        options.theme.cursor.hash(hash);
        options.theme.truecolor.hash(hash);
    });
    let bottom = fingerprint(|hash| {
        viewport.height.hash(hash);
        viewport.width.hash(hash);
        message.hash(hash);
        options.action_bar.hash(hash);
        options.status_role.hash(hash);
        options.status_theme.hash(hash);
        options.status_path.hash(hash);
        options.status_filename.hash(hash);
        options.status_selection.hash(hash);
    });
    let overlay = fingerprint(|hash| {
        options.emoji_picker.is_some().hash(hash);
        if let Some(picker) = options.emoji_picker {
            cursor.hash(hash);
            picker.rows.hash(hash);
            picker.selected.hash(hash);
            viewport.width.hash(hash);
            viewport.height.hash(hash);
            options.action_bar.is_some().hash(hash);
            options.theme.hash(hash);
        }
    });
    (header, bottom, overlay)
}

pub(super) fn empty_row_fingerprint(options: RenderOptions<'_>) -> u64 {
    fingerprint(|hash| {
        options.theme.hash(hash);
        usize::MAX.hash(hash);
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn row_fingerprint(
    options: RenderOptions<'_>,
    document_row: usize,
    start_col: usize,
    content_fingerprint: Option<u64>,
    visible_byte_len: usize,
    visible_scalar_len: usize,
    wrapped: bool,
    line_end: bool,
) -> u64 {
    fingerprint(|hash| {
        document_row.hash(hash);
        start_col.hash(hash);
        content_fingerprint.hash(hash);
        visible_byte_len.hash(hash);
        visible_scalar_len.hash(hash);
        wrapped.hash(hash);
        line_end.hash(hash);
        options.theme.hash(hash);
        options.syntax.hash(hash);
        options.surface.hash(hash);
        options.line_numbers.hash(hash);
        options.whitespace.hash(hash);

        let highlight = style::visible_highlight(
            options.highlight,
            document_row,
            start_col,
            visible_scalar_len,
        );
        highlight.hash(hash);
        if highlight.is_some() {
            options.highlight_kind.hash(hash);
        }
        let mut lint_count = 0_usize;
        for range in options
            .lint_ranges
            .map(|ranges| style::ranges_for_row(ranges, document_row))
            .unwrap_or_default()
        {
            if let Some(local) =
                style::visible_highlight(Some(*range), document_row, start_col, visible_scalar_len)
            {
                local.hash(hash);
                lint_count = lint_count.saturating_add(1);
            }
        }
        lint_count.hash(hash);
        let mut added_count = 0_usize;
        if let Some(changes) = options.external_changes {
            for range in style::ranges_for_row(changes.added_ranges, document_row) {
                if let Some(local) = style::visible_highlight(
                    Some(*range),
                    document_row,
                    start_col,
                    visible_scalar_len,
                ) {
                    local.hash(hash);
                    added_count = added_count.saturating_add(1);
                }
            }
        }
        added_count.hash(hash);
        let mut changed_count = 0_usize;
        if let Some(changes) = options.external_changes {
            for range in style::ranges_for_row(changes.changed_ranges, document_row) {
                if let Some(local) = style::visible_highlight(
                    Some(*range),
                    document_row,
                    start_col,
                    visible_scalar_len,
                ) {
                    local.hash(hash);
                    changed_count = changed_count.saturating_add(1);
                }
            }
        }
        changed_count.hash(hash);
        let marker = options.external_changes.and_then(|changes| {
            (!wrapped || start_col == 0)
                .then(|| {
                    changes
                        .markers
                        .binary_search_by_key(&document_row, |marker| marker.line)
                        .ok()
                        .and_then(|index| changes.markers.get(index))
                })
                .flatten()
        });
        marker.hash(hash);
        options.presentation.is_some().hash(hash);
        if let Some(presentation) = options.presentation {
            let mut span_count = 0_usize;
            for span in presentation.annotations.spans(document_row) {
                if let Some((start, end)) =
                    local_intersection(span.start, span.end, start_col, visible_scalar_len)
                {
                    start.hash(hash);
                    end.hash(hash);
                    span.style.hash(hash);
                    span_count = span_count.saturating_add(1);
                }
            }
            span_count.hash(hash);
            let mut link_count = 0_usize;
            for link in presentation.annotations.links(document_row) {
                if let Some((start, end)) =
                    local_intersection(link.start, link.end, start_col, visible_scalar_len)
                {
                    start.hash(hash);
                    end.hash(hash);
                    link.destination.hash(hash);
                    link_count = link_count.saturating_add(1);
                }
            }
            link_count.hash(hash);
        }
    })
}

pub(super) fn content_fingerprint(content: &str) -> u64 {
    fingerprint(|hash| content.hash(hash))
}

fn local_intersection(
    range_start: usize,
    range_end: usize,
    start_col: usize,
    visible_scalar_len: usize,
) -> Option<(usize, usize)> {
    let visible_end = start_col.saturating_add(visible_scalar_len);
    let start = range_start.max(start_col);
    let end = range_end.min(visible_end);
    (start < end).then_some((
        start.saturating_sub(start_col),
        end.saturating_sub(start_col),
    ))
}

fn fingerprint(write: impl FnOnce(&mut DefaultHasher)) -> u64 {
    let mut hash = DefaultHasher::new();
    write(&mut hash);
    hash.finish()
}

use std::ops::Range;

#[derive(Clone, Copy)]
pub(super) struct GridMetrics {
    pub(super) columns: usize,
    pub(super) card_height: f32,
    pub(super) row_gap: f32,
    pub(super) header_height: f32,
    pub(super) section_gap: f32,
}

struct SectionLayout {
    index: usize,
    indices: Range<usize>,
    top: f32,
    bottom: f32,
    rows: usize,
}

pub(super) struct GroupedGridLayout {
    sections: Vec<SectionLayout>,
    metrics: GridMetrics,
    height: f32,
}

#[derive(Debug, PartialEq)]
pub(super) enum GridRowKind {
    Header {
        section: usize,
    },
    Cards {
        section: usize,
        indices: Range<usize>,
    },
}

#[derive(Debug, PartialEq)]
pub(super) struct GridRow {
    pub(super) top: f32,
    pub(super) height: f32,
    pub(super) kind: GridRowKind,
}

impl GroupedGridLayout {
    pub(super) fn new(
        sections: impl IntoIterator<Item = (Range<usize>, bool)>,
        metrics: GridMetrics,
    ) -> Self {
        let metrics = GridMetrics {
            columns: metrics.columns.max(1),
            card_height: metrics.card_height.max(1.0),
            row_gap: metrics.row_gap.max(0.0),
            header_height: metrics.header_height.max(1.0),
            section_gap: metrics.section_gap.max(0.0),
        };
        let mut height = 0.0;
        let sections = sections
            .into_iter()
            .enumerate()
            .map(|(index, (indices, expanded))| {
                let count = indices.end.saturating_sub(indices.start);
                let rows = if expanded {
                    count.div_ceil(metrics.columns)
                } else {
                    0
                };
                let top = height;
                height += metrics.header_height
                    + rows as f32 * (metrics.card_height + metrics.row_gap)
                    + metrics.section_gap;
                SectionLayout {
                    index,
                    indices,
                    top,
                    bottom: height,
                    rows,
                }
            })
            .collect();
        Self {
            sections,
            metrics,
            height,
        }
    }

    pub(super) fn height(&self) -> f32 {
        self.height
    }

    pub(super) fn visible_rows(&self, viewport: Range<f32>) -> Vec<GridRow> {
        let mut visible = Vec::new();
        if !viewport.start.is_finite()
            || !viewport.end.is_finite()
            || viewport.end <= viewport.start
        {
            return visible;
        }
        let start = viewport.start.max(0.0);
        let end = viewport.end.min(self.height);
        if end <= start {
            return visible;
        }
        let first_section = self
            .sections
            .partition_point(|section| section.bottom <= start);
        let row_stride = self.metrics.card_height + self.metrics.row_gap;
        for section in &self.sections[first_section..] {
            if section.top >= end {
                break;
            }
            let cards_top = section.top + self.metrics.header_height;
            if cards_top > start {
                visible.push(GridRow {
                    top: section.top,
                    height: self.metrics.header_height,
                    kind: GridRowKind::Header {
                        section: section.index,
                    },
                });
            }

            let first_row = ((start - cards_top).max(0.0) / row_stride).floor() as usize;
            let last_row =
                (((end - cards_top).max(0.0) / row_stride).ceil() as usize).min(section.rows);
            for row in first_row..last_row {
                let top = cards_top + row as f32 * row_stride;
                if top + self.metrics.card_height <= start || top >= end {
                    continue;
                }
                let first = section.indices.start + row * self.metrics.columns;
                let last = first
                    .saturating_add(self.metrics.columns)
                    .min(section.indices.end);
                visible.push(GridRow {
                    top,
                    height: self.metrics.card_height,
                    kind: GridRowKind::Cards {
                        section: section.index,
                        indices: first..last,
                    },
                });
            }
        }
        visible
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> GridMetrics {
        GridMetrics {
            columns: 3,
            card_height: 100.0,
            row_gap: 5.0,
            header_height: 28.0,
            section_gap: 5.0,
        }
    }

    fn card_indices(rows: &[GridRow]) -> Vec<usize> {
        rows.iter()
            .flat_map(|row| match &row.kind {
                GridRowKind::Cards { indices, .. } => indices.clone(),
                GridRowKind::Header { .. } => 0..0,
            })
            .collect()
    }

    #[test]
    fn variable_height_sections_preserve_headers_and_global_indices() {
        let layout =
            GroupedGridLayout::new([(10..17, true), (40..42, false), (90..94, true)], metrics());
        assert_eq!(layout.height(), 624.0);
        let rows = layout.visible_rows(0.0..700.0);
        assert_eq!(rows[0].kind, GridRowKind::Header { section: 0 });
        assert_eq!(rows[4].kind, GridRowKind::Header { section: 1 });
        assert_eq!(rows[4].top, 348.0);
        assert_eq!(rows[5].kind, GridRowKind::Header { section: 2 });
        assert_eq!(rows[5].top, 381.0);
        assert_eq!(
            card_indices(&rows),
            (10..17).chain(90..94).collect::<Vec<_>>()
        );
    }

    #[test]
    fn collapsed_and_empty_sections_keep_their_headers_without_cards() {
        let layout = GroupedGridLayout::new([(0..1000, false), (10..10, true)], metrics());
        let rows = layout.visible_rows(0.0..100.0);
        assert_eq!(layout.height(), 66.0);
        assert_eq!(rows.len(), 2);
        assert!(card_indices(&rows).is_empty());
        assert_eq!(rows[1].kind, GridRowKind::Header { section: 1 });
    }

    #[test]
    fn viewport_omits_offscreen_cards_and_spacing_only_rows() {
        let layout = GroupedGridLayout::new([(10..17, true)], metrics());
        assert!(layout.visible_rows(128.0..133.0).is_empty());
        let rows = layout.visible_rows(133.0..233.0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].top, 133.0);
        assert_eq!(card_indices(&rows), vec![13, 14, 15]);
        assert!(layout.visible_rows(500.0..600.0).is_empty());
        assert!(layout.visible_rows(10.0..10.0).is_empty());
        assert!(layout.visible_rows(f32::NAN..100.0).is_empty());
    }

    #[test]
    fn partial_rows_and_final_columns_are_visible_without_index_drift() {
        let layout = GroupedGridLayout::new([(5..12, true)], metrics());
        let rows = layout.visible_rows(230.0..245.0);
        assert_eq!(rows.len(), 2);
        assert_eq!(card_indices(&rows), vec![8, 9, 10, 11]);
        assert_eq!(rows[1].top, 238.0);
        assert_eq!(
            rows[1].kind,
            GridRowKind::Cards {
                section: 0,
                indices: 11..12
            }
        );
    }

    #[test]
    fn column_changes_reflow_rows_without_changing_global_indices() {
        let mut metrics = metrics();
        metrics.columns = 2;
        let layout = GroupedGridLayout::new([(5..12, true)], metrics);
        assert_eq!(layout.height(), 453.0);
        assert_eq!(
            card_indices(&layout.visible_rows(0.0..1000.0)),
            (5..12).collect::<Vec<_>>()
        );

        metrics.columns = 0;
        let narrow = GroupedGridLayout::new([(5..7, true)], metrics);
        assert_eq!(narrow.visible_rows(0.0..1000.0).len(), 3);
    }

    #[test]
    fn huge_groups_only_materialize_visible_rows() {
        let layout = GroupedGridLayout::new([(0..1_000_000, true)], metrics());
        assert_eq!(layout.sections.len(), 1);
        let rows = layout.visible_rows(1_000_000.0..1_000_500.0);
        assert!(rows.len() <= 6);
        assert!(!rows.is_empty());
        assert!(card_indices(&rows).iter().all(|index| *index > 20_000));
    }

    #[test]
    fn header_only_galleries_jump_to_visible_sections() {
        let layout = GroupedGridLayout::new((0..10_000).map(|_| (0..0, false)), metrics());
        let rows = layout.visible_rows(33_000.0..33_028.0);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, GridRowKind::Header { section: 1000 });
    }

    #[test]
    fn empty_gallery_has_no_height_or_rows() {
        let layout = GroupedGridLayout::new([], metrics());
        assert_eq!(layout.height(), 0.0);
        assert!(layout.visible_rows(0.0..100.0).is_empty());
    }
}

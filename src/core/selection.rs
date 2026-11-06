use std::collections::BTreeSet;
use std::ops::Range;

/// Query row geometry without restricting the answer to the current viewport.
/// Binary search keeps a 100,000-row table from enumerating offscreen views.
pub fn intersecting_rows(
    top: f64,
    bottom: f64,
    count: usize,
    mut bounds: impl FnMut(usize) -> (f64, f64),
) -> Range<usize> {
    if bottom <= top {
        return 0..0;
    }
    let (mut low, mut high) = (0, count);
    while low < high {
        let mid = low + (high - low) / 2;
        if bounds(mid).1 <= top {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let first = low;
    high = count;
    while low < high {
        let mid = low + (high - low) / 2;
        if bounds(mid).0 < bottom {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    first..low
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    Replace,
    Add,
    Toggle,
}

impl SelectionMode {
    pub fn from_modifiers(shift: bool, primary: bool) -> Self {
        if primary {
            Self::Toggle
        } else if shift {
            Self::Add
        } else {
            Self::Replace
        }
    }

    /// Always compare to the selection at mouse-down, so crossing the same
    /// item repeatedly does not repeatedly toggle it.
    pub fn apply(self, initial: &BTreeSet<usize>, hits: BTreeSet<usize>) -> BTreeSet<usize> {
        match self {
            Self::Replace => hits,
            Self::Add => initial.union(&hits).copied().collect(),
            Self::Toggle => initial.symmetric_difference(&hits).copied().collect(),
        }
    }
}

/// Scroll near either edge, including when the pointer is held still there.
pub fn edge_scroll(position: f64, start: f64, length: f64) -> f64 {
    let margin = 24.0_f64.min(length / 3.0);
    if position < start + margin {
        -((start + margin - position) * 0.8).clamp(2.0, 48.0)
    } else if position > start + length - margin {
        ((position - start - length + margin) * 0.8).clamp(2.0, 48.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rows(values: &[usize]) -> BTreeSet<usize> {
        values.iter().copied().collect()
    }

    #[test]
    fn scrolling_keeps_offscreen_rows_and_exact_row_boundaries() {
        let geometry = |i: usize| (i as f64 * 30., (i + 1) as f64 * 30.);
        assert_eq!(intersecting_rows(15., 507., 120, geometry), 0..17);
        assert_eq!(intersecting_rows(30., 60., 120, geometry), 1..2);
        assert_eq!(intersecting_rows(-90., -10., 120, geometry), 0..0);
        assert_eq!(intersecting_rows(4000., 5000., 120, geometry), 120..120);
        assert_eq!(intersecting_rows(30., 30., 120, geometry), 0..0);
    }
    #[test]
    fn hundred_thousand_rows_need_only_logarithmic_geometry_queries() {
        let mut queries = 0;
        let range = intersecting_rows(10., 3_000_000., 100_000, |i| {
            queries += 1;
            (i as f64 * 30., (i + 1) as f64 * 30.)
        });
        assert_eq!(range, 0..100_000);
        assert!(queries <= 36);
    }

    #[test]
    fn replacement_shrinks_and_clears_as_the_rectangle_moves() {
        let initial = rows(&[9]);
        assert_eq!(
            SelectionMode::Replace.apply(&initial, rows(&[1, 2, 3])),
            rows(&[1, 2, 3])
        );
        assert_eq!(
            SelectionMode::Replace.apply(&initial, rows(&[2])),
            rows(&[2])
        );
        assert!(SelectionMode::Replace.apply(&initial, rows(&[])).is_empty());
    }
    #[test]
    fn additive_and_toggle_drag_use_original_selection() {
        let initial = rows(&[1, 9]);
        assert_eq!(
            SelectionMode::Add.apply(&initial, rows(&[1, 2])),
            rows(&[1, 2, 9])
        );
        assert_eq!(
            SelectionMode::Toggle.apply(&initial, rows(&[1, 2])),
            rows(&[2, 9])
        );
        assert_eq!(
            SelectionMode::Toggle.apply(&initial, rows(&[2])),
            rows(&[1, 2, 9])
        );
        assert_eq!(SelectionMode::Toggle.apply(&initial, rows(&[])), initial);
    }
    #[test]
    fn stationary_edge_pointer_scrolls_in_both_directions_and_clamps_speed() {
        assert!(edge_scroll(20., 20., 500.) < 0.);
        assert!(edge_scroll(518., 20., 500.) > 0.);
        assert_eq!(edge_scroll(200., 20., 500.), 0.);
        assert_eq!(edge_scroll(-1000., 20., 500.), -48.);
        assert_eq!(edge_scroll(2000., 20., 500.), 48.);
    }
}

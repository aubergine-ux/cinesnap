//! Cuts and the mapping between source time and output time.

use crate::project::Cut;

/// The parts of the recording that survive cutting, as sorted, disjoint
/// `(start_ms, end_ms)` ranges in source time.
#[derive(Clone, Debug, PartialEq)]
pub struct Kept {
    pub ranges: Vec<(f64, f64)>,
}

impl Kept {
    pub fn new(duration_ms: f64, cuts: &[Cut]) -> Self {
        let mut cuts: Vec<(f64, f64)> = cuts
            .iter()
            .map(|c| (c.start_ms.max(0.0), c.end_ms.min(duration_ms)))
            .filter(|(s, e)| e > s)
            .collect();
        cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut ranges = Vec::new();
        let mut pos = 0.0;
        for (s, e) in cuts {
            if s > pos {
                ranges.push((pos, s));
            }
            pos = e.max(pos);
        }
        if duration_ms > pos {
            ranges.push((pos, duration_ms));
        }
        Self { ranges }
    }

    pub fn duration_ms(&self) -> f64 {
        self.ranges.iter().map(|(s, e)| e - s).sum()
    }

    /// Source time shown at the given output time (clamped to the ends).
    pub fn out_to_src(&self, out_ms: f64) -> f64 {
        let mut acc = 0.0;
        for &(s, e) in &self.ranges {
            let len = e - s;
            if out_ms < acc + len {
                return s + (out_ms - acc).max(0.0);
            }
            acc += len;
        }
        self.ranges.last().map_or(0.0, |r| r.1)
    }

    /// Output time at which a source time appears, or `None` if it was cut.
    pub fn src_to_out(&self, src_ms: f64) -> Option<f64> {
        let mut acc = 0.0;
        for &(s, e) in &self.ranges {
            if src_ms >= s && src_ms < e {
                return Some(acc + src_ms - s);
            }
            acc += e - s;
        }
        None
    }

    pub fn contains(&self, src_ms: f64) -> bool {
        self.src_to_out(src_ms).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut(a: f64, b: f64) -> Cut {
        Cut { start_ms: a, end_ms: b }
    }

    #[test]
    fn no_cuts() {
        let k = Kept::new(1000.0, &[]);
        assert_eq!(k.ranges, vec![(0.0, 1000.0)]);
        assert_eq!(k.out_to_src(250.0), 250.0);
    }

    #[test]
    fn overlapping_and_unsorted_cuts_merge() {
        let k = Kept::new(1000.0, &[cut(600.0, 700.0), cut(100.0, 300.0), cut(250.0, 400.0), cut(900.0, 5000.0)]);
        assert_eq!(k.ranges, vec![(0.0, 100.0), (400.0, 600.0), (700.0, 900.0)]);
        assert_eq!(k.duration_ms(), 500.0);
        assert_eq!(k.out_to_src(50.0), 50.0);
        assert_eq!(k.out_to_src(100.0), 400.0);
        assert_eq!(k.out_to_src(350.0), 750.0);
        assert_eq!(k.out_to_src(9999.0), 900.0);
        assert_eq!(k.src_to_out(450.0), Some(150.0));
        assert_eq!(k.src_to_out(200.0), None);
    }

    #[test]
    fn everything_cut() {
        let k = Kept::new(1000.0, &[cut(0.0, 1000.0)]);
        assert!(k.ranges.is_empty());
        assert_eq!(k.duration_ms(), 0.0);
        assert_eq!(k.out_to_src(10.0), 0.0);
    }
}

use std::cmp::{max, min, Ordering};
use std::fmt;
use std::ops::{Range, RangeInclusive};

use chrono::NaiveDateTime;
use opening_hours_syntax::rules::RuleKind;
use opening_hours_syntax::sorted_vec::UniqueSortedVec;

// DateTimeRange

#[non_exhaustive]
#[derive(Clone, Eq, PartialEq)]
pub struct DateTimeRange<'c> {
    pub range: Range<NaiveDateTime>,
    pub kind: RuleKind,
    pub comments: UniqueSortedVec<&'c str>,
}

impl fmt::Debug for DateTimeRange<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DateTimeRange")
            .field("range", &self.range)
            .field("kind", &self.kind)
            .field("comments", &self.comments)
            .finish()
    }
}

impl<'c> DateTimeRange<'c> {
    pub(crate) fn new_with_sorted_comments(
        range: Range<NaiveDateTime>,
        kind: RuleKind,
        comments: UniqueSortedVec<&'c str>,
    ) -> Self {
        Self { range, kind, comments }
    }

    pub fn comments(&self) -> &[&'c str] {
        &self.comments
    }

    pub fn into_comments(self) -> UniqueSortedVec<&'c str> {
        self.comments
    }
}

// WrappingRange

pub(crate) trait WrappingRange<T> {
    fn wrapping_contains(&self, elt: &T) -> bool;
}

impl<T: PartialOrd> WrappingRange<T> for RangeInclusive<T> {
    fn wrapping_contains(&self, elt: &T) -> bool {
        if self.start() <= self.end() {
            self.contains(elt)
        } else {
            self.start() <= elt || elt <= self.end()
        }
    }
}

// RangeCompare

pub(crate) trait RangeCompare<T> {
    fn compare(&self, elt: &T) -> Ordering;
}

impl<T: PartialOrd> RangeCompare<T> for RangeInclusive<T> {
    fn compare(&self, elt: &T) -> Ordering {
        debug_assert!(self.start() <= self.end());

        if elt < self.start() {
            Ordering::Less
        } else if elt > self.end() {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

impl<T: PartialOrd> RangeCompare<T> for Range<T> {
    fn compare(&self, elt: &T) -> Ordering {
        debug_assert!(self.start <= self.end);

        if elt < &self.start {
            Ordering::Less
        } else if elt >= &self.end {
            Ordering::Greater
        } else {
            Ordering::Equal
        }
    }
}

// Range operations

pub(crate) fn time_ranges_union<T: Ord>(
    ranges: impl Iterator<Item = Range<T>>,
) -> impl Iterator<Item = Range<T>> {
    // TODO: we could gain performance by ensuring that range iterators are
    //       always sorted.
    let mut ranges: Vec<_> = ranges.collect();
    ranges.sort_unstable_by(|r1, r2| r1.start.cmp(&r2.start));

    // Get ranges by increasing start
    let mut ranges = ranges.into_iter();
    let mut current_opt = ranges.next();

    std::iter::from_fn(move || {
        if let Some(ref mut current) = current_opt {
            #[allow(clippy::while_let_on_iterator)]
            while let Some(item) = ranges.next() {
                if current.end >= item.start {
                    // The two intervals intersect with each other
                    if item.end > current.end {
                        current.end = item.end;
                    }
                } else {
                    return Some(current_opt.replace(item).unwrap());
                }
            }

            Some(current_opt.take().unwrap())
        } else {
            None
        }
    })
}

pub(crate) fn range_intersection<T: Ord>(range_1: Range<T>, range_2: Range<T>) -> Option<Range<T>> {
    let result = max(range_1.start, range_2.start)..min(range_1.end, range_2.end);

    if result.start < result.end {
        Some(result)
    } else {
        None
    }
}

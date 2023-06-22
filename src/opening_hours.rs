use std::boxed::Box;
use std::cmp::{max, min};
use std::convert::TryInto;
use std::iter::{empty, Peekable};

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use once_cell::sync::Lazy;

use compact_calendar::CompactCalendar;
use opening_hours_syntax::extended_time::ExtendedTime;
use opening_hours_syntax::rules::{RuleKind, RuleOperator, RuleSequence};

use crate::context::{Context, REGION_HOLIDAYS};
use crate::date_filter::DateFilter;
use crate::error::{Error, Result};
use crate::localize::{Localize, LocalizeWithTz, NoLocation};
use crate::schedule::{Schedule, TimeRange};
use crate::time_filter::{time_selector_intervals_at, time_selector_intervals_at_next_day};
use crate::DateTimeRange;

/// The upper bound of dates handled by specification
pub static DATE_LIMIT: Lazy<NaiveDateTime> = Lazy::new(|| {
    NaiveDateTime::new(
        NaiveDate::from_ymd_opt(10_000, 1, 1).expect("invalid max date bound"),
        NaiveTime::from_hms_opt(0, 0, 0).expect("invalid max time bound"),
    )
});

// OpeningHours

#[derive(Clone)]
pub struct OpeningHours<L = NoLocation> {
    /// Rules describing opening hours
    rules: Vec<RuleSequence>,
    /// Execution context for opening hours
    ctx: Context<L>,
}

impl OpeningHours<NoLocation> {
    /// Init a new TimeDomain with the given set of Rules.
    pub fn parse(data: &str) -> Result<OpeningHours<NoLocation>> {
        Ok(OpeningHours {
            rules: opening_hours_syntax::parse(data)?,
            ctx: Context::default(),
        })
    }

    // TODO: doc
    #[cfg(feature = "localize")]
    pub fn try_with_coord(
        self,
        lat: f64,
        lon: f64,
    ) -> Result<OpeningHours<<NoLocation as Localize>::WithCoordInferTz>> {
        todo!()
    }
}

impl<L: Localize> OpeningHours<L> {
    /// Get the list of all loaded public holidays.
    pub fn holidays(&self) -> &'static CompactCalendar {
        self.ctx.holidays
    }

    /// Replace loaded holidays with known holidays for the given region. If
    /// the region is not existing, no holiday will be loaded.
    ///
    /// ```
    /// use opening_hours::OpeningHours;
    ///
    /// let oh = OpeningHours::parse("24/7").unwrap();
    /// assert_eq!(oh.holidays().count(), 0);
    /// assert_ne!(oh.with_region("FR").unwrap().holidays().count(), 0);
    /// ```
    pub fn with_region(mut self, region: &str) -> Result<Self> {
        self.ctx.holidays = REGION_HOLIDAYS
            .get(region.to_uppercase().as_str())
            .ok_or_else(|| Error::RegionNotFound(region.to_string()))?;

        Ok(self)
    }

    // Low level implementations.
    //
    // Following functions are used to build the TimeDomainIterator which is
    // used to implement all other functions.
    //
    // This means that performances matters a lot for these functions and it
    // would be relevant to focus on optimisations to this regard.

    /// Provide a lower bound to the next date when a different set of rules
    /// could match.
    fn next_change_hint(&self, date: NaiveDate) -> Option<NaiveDate> {
        self.rules
            .iter()
            .map(|rule| rule.day_selector.next_change_hint(date, self.holidays()))
            .min()
            .flatten()
    }

    // TODO: doc
    pub fn schedule_at(&self, date: NaiveDate) -> Schedule {
        let mut prev_match = false;
        let mut prev_eval = None;

        for rules_seq in &self.rules {
            let curr_match = rules_seq.day_selector.filter(date, self.holidays());
            let curr_eval = rule_sequence_schedule_at(&self.ctx, rules_seq, date, self.holidays());

            let (new_match, new_eval) = match rules_seq.operator {
                RuleOperator::Normal => (
                    curr_match || prev_match,
                    if curr_match {
                        curr_eval
                    } else {
                        prev_eval.or(curr_eval)
                    },
                ),
                RuleOperator::Additional => (
                    prev_match || curr_match,
                    match (prev_eval, curr_eval) {
                        (Some(prev), Some(curr)) => Some(prev.addition(curr)),
                        (prev, curr) => prev.or(curr),
                    },
                ),
                RuleOperator::Fallback => {
                    if prev_match {
                        (prev_match, prev_eval)
                    } else {
                        (curr_match, curr_eval)
                    }
                }
            };

            prev_match = new_match;
            prev_eval = new_eval;
        }

        prev_eval.unwrap_or_else(Schedule::empty)
    }

    // TODO: doc
    pub fn iter_from(
        &self,
        from: NaiveDateTime,
    ) -> Result<impl Iterator<Item = DateTimeRange> + '_> {
        self.iter_range(from, *DATE_LIMIT)
    }

    // TODO: doc
    pub fn iter_range(
        &self,
        from: NaiveDateTime,
        to: NaiveDateTime,
    ) -> Result<impl Iterator<Item = DateTimeRange> + '_> {
        if from >= *DATE_LIMIT {
            Err(Error::DateLimitExceeded(from))
        } else if to > *DATE_LIMIT {
            Err(Error::DateLimitExceeded(to))
        } else {
            Ok(TimeDomainIterator::new(self, from, to)
                .take_while(move |dtr| dtr.range.start < to)
                .map(move |dtr| {
                    let start = max(dtr.range.start, from);
                    let end = min(dtr.range.end, to);
                    DateTimeRange::new_with_sorted_comments(start..end, dtr.kind, dtr.comments)
                }))
        }
    }

    // High level implementations

    // TODO: doc
    pub fn next_change(&self, current_time: NaiveDateTime) -> Result<NaiveDateTime> {
        Ok(self
            .iter_from(current_time)?
            .next()
            .map(|dtr| dtr.range.end)
            .unwrap_or(*DATE_LIMIT))
    }

    // TODO: doc
    pub fn state(&self, current_time: NaiveDateTime) -> Result<RuleKind> {
        Ok(self
            .iter_range(current_time, current_time + Duration::minutes(1))?
            .next()
            .map(|dtr| dtr.kind)
            .unwrap_or(RuleKind::Unknown))
    }

    // TODO: doc
    pub fn is_open(&self, current_time: NaiveDateTime) -> bool {
        matches!(self.state(current_time), Ok(RuleKind::Open))
    }

    // TODO: doc
    pub fn is_closed(&self, current_time: NaiveDateTime) -> bool {
        matches!(self.state(current_time), Ok(RuleKind::Closed))
    }

    // TODO: doc
    pub fn is_unknown(&self, current_time: NaiveDateTime) -> bool {
        matches!(
            self.state(current_time),
            Err(Error::DateLimitExceeded(_)) | Ok(RuleKind::Unknown)
        )
    }

    // TODO: doc
    pub fn intervals(
        &self,
        from: NaiveDateTime,
        to: NaiveDateTime,
    ) -> Result<impl Iterator<Item = DateTimeRange>> {
        Ok(self
            .iter_from(from)?
            .take_while(move |dtr| dtr.range.start < to)
            .map(move |dtr| {
                let start = max(dtr.range.start, from);
                let end = min(dtr.range.end, to);
                DateTimeRange::new_with_sorted_comments(start..end, dtr.kind, dtr.comments)
            }))
    }
}

impl<L: Localize> OpeningHours<L> {
    // TODO: doc
    #[cfg(feature = "localize")]
    pub fn with_tz<Tz: TimeZone>(self, tz: Tz) -> OpeningHours<L::WithTz<Tz>> {
        OpeningHours {
            rules: self.rules,
            ctx: Context {
                holidays: self.ctx.holidays,
                localize: self.ctx.localize.with_tz(tz),
            },
        }
    }

    // TODO: doc
    #[cfg(feature = "localize")]
    fn try_with_coord_infer_tz(
        self,
        lat: f64,
        lon: f64,
    ) -> Result<OpeningHours<L::WithCoordInferTz>> {
        Ok(OpeningHours {
            rules: self.rules,
            ctx: Context {
                holidays: self.ctx.holidays,
                localize: self.ctx.localize.try_with_coord_infer_tz(lat, lon)?,
            },
        })
    }
}

#[cfg(feature = "localize")]
impl<L: LocalizeWithTz> OpeningHours<L> {
    // TODO: doc
    pub fn with_coords(self, lat: f64, lon: f64) -> OpeningHours<L::WithCoord> {
        OpeningHours {
            rules: self.rules,
            ctx: Context {
                holidays: self.ctx.holidays,
                localize: self.ctx.localize.with_coord(lat, lon),
            },
        }
    }
}

fn rule_sequence_schedule_at<'s, L: Localize>(
    ctx: &'s Context<L>,
    rule_sequence: &'s RuleSequence,
    date: NaiveDate,
    holiday: &CompactCalendar,
) -> Option<Schedule<'s>> {
    let from_today = Some(date)
        .filter(|date| rule_sequence.day_selector.filter(*date, holiday))
        .map(|date| time_selector_intervals_at(ctx, &rule_sequence.time_selector, date))
        .map(|rgs| Schedule::from_ranges(rgs, rule_sequence.kind, rule_sequence.comments.to_ref()));

    let from_yesterday = (date.pred_opt())
        .filter(|prev| rule_sequence.day_selector.filter(*prev, holiday))
        .map(|prev| time_selector_intervals_at_next_day(ctx, &rule_sequence.time_selector, prev))
        .map(|rgs| Schedule::from_ranges(rgs, rule_sequence.kind, rule_sequence.comments.to_ref()));

    match (from_today, from_yesterday) {
        (Some(sched_1), Some(sched_2)) => Some(sched_1.addition(sched_2)),
        (today, yesterday) => today.or(yesterday),
    }
}

// TimeDomainIterator

// TODO: lacks documentation
pub struct TimeDomainIterator<'d, L: Localize> {
    opening_hours: &'d OpeningHours<L>,
    curr_date: NaiveDate,
    curr_schedule: Peekable<Box<dyn Iterator<Item = TimeRange<'d>> + 'd>>,
    end_datetime: NaiveDateTime,
}

impl<'d, L: Localize> TimeDomainIterator<'d, L> {
    pub(crate) fn new(
        opening_hours: &'d OpeningHours<L>,
        start_datetime: NaiveDateTime,
        end_datetime: NaiveDateTime,
    ) -> Self {
        let start_date = start_datetime.date();
        let start_time = start_datetime.time().into();

        let mut curr_schedule = {
            if start_datetime < end_datetime {
                opening_hours.schedule_at(start_date).into_iter_filled()
            } else {
                Box::new(empty())
            }
        }
        .peekable();

        while curr_schedule
            .peek()
            .map(|tr| !tr.range.contains(&start_time))
            .unwrap_or(false)
        {
            curr_schedule.next();
        }

        Self {
            opening_hours,
            curr_date: start_date,
            curr_schedule,
            end_datetime,
        }
    }

    fn consume_until_next_kind(&mut self, curr_kind: RuleKind) {
        while self.curr_schedule.peek().map(|tr| tr.kind) == Some(curr_kind) {
            self.curr_schedule.next();

            if self.curr_schedule.peek().is_none() {
                self.curr_date = self
                    .opening_hours
                    .next_change_hint(self.curr_date)
                    .unwrap_or_else(|| self.curr_date.succ_opt().expect("reached invalid date"));

                if self.curr_date < self.end_datetime.date() {
                    self.curr_schedule = self
                        .opening_hours
                        .schedule_at(self.curr_date)
                        .into_iter_filled()
                        .peekable()
                }
            }
        }
    }
}

impl<'d, L: Localize> Iterator for TimeDomainIterator<'d, L> {
    type Item = DateTimeRange<'d>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(curr_tr) = self.curr_schedule.peek().cloned() {
            let start = NaiveDateTime::new(
                self.curr_date,
                curr_tr
                    .range
                    .start
                    .try_into()
                    .expect("got invalid time from schedule"),
            );

            self.consume_until_next_kind(curr_tr.kind);

            let end_date = self.curr_date;
            let end_time = self
                .curr_schedule
                .peek()
                .map(|tr| tr.range.start)
                .unwrap_or_else(|| ExtendedTime::new(0, 0));

            let end = std::cmp::min(
                self.end_datetime,
                NaiveDateTime::new(
                    end_date,
                    end_time.try_into().expect("got invalid time from schedule"),
                ),
            );

            Some(DateTimeRange::new_with_sorted_comments(
                start..end,
                curr_tr.kind,
                curr_tr.comments,
            ))
        } else {
            None
        }
    }
}

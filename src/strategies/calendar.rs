//! Calendar anomalies: does *when* you hold matter?
//!
//! Decisions are made at today's close for tomorrow, using only the calendar.
//! "Next trading day" is approximated as the next weekday (holidays ignored).

use crate::data::Bar;
use crate::strategy::{Spec, SymbolStrategy};
use chrono::{Datelike, Duration, NaiveDate, Weekday};

fn next_weekday(d: NaiveDate) -> NaiveDate {
    let mut n = d + Duration::days(1);
    while matches!(n.weekday(), Weekday::Sat | Weekday::Sun) {
        n += Duration::days(1);
    }
    n
}

#[cfg(test)]
fn days_in_month(d: NaiveDate) -> u32 {
    let (y, m) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    NaiveDate::from_ymd_opt(y, m, 1)
        .unwrap()
        .pred_opt()
        .unwrap()
        .day()
}

/// Weekdays remaining in the month after `d` (inclusive of later days only).
fn weekdays_left(d: NaiveDate) -> u32 {
    let mut n = 0;
    let mut x = d;
    loop {
        x = next_weekday(x);
        if x.month() != d.month() {
            return n;
        }
        n += 1;
    }
}

/// Weekday index within the month (1 = first weekday of month).
fn weekday_of_month(d: NaiveDate) -> u32 {
    (1..=d.day())
        .filter(|k| {
            !matches!(
                d.with_day(*k).unwrap().weekday(),
                Weekday::Sat | Weekday::Sun
            )
        })
        .count() as u32
}

/// Long over the last `before` and first `after` trading days of each month.
struct TurnOfMonth {
    before: u32,
    after: u32,
}
impl SymbolStrategy for TurnOfMonth {
    fn next(&mut self, d: NaiveDate, _: &Bar) -> f64 {
        let t = next_weekday(d);
        let in_window = if t.month() != d.month() {
            self.after >= 1
        } else {
            weekdays_left(t) < self.before || weekday_of_month(t) <= self.after
        };
        if in_window {
            1.0
        } else {
            0.0
        }
    }
}

/// Seasonal: long only during the given months (1-12).
struct Months(Vec<u32>);
impl SymbolStrategy for Months {
    fn next(&mut self, d: NaiveDate, _: &Bar) -> f64 {
        if self.0.contains(&next_weekday(d).month()) {
            1.0
        } else {
            0.0
        }
    }
}

/// Hold only on one weekday.
struct DayOfWeek(Weekday);
impl SymbolStrategy for DayOfWeek {
    fn next(&mut self, d: NaiveDate, _: &Bar) -> f64 {
        if next_weekday(d).weekday() == self.0 {
            1.0
        } else {
            0.0
        }
    }
}

pub fn specs() -> Vec<Spec> {
    let mut v = Vec::new();
    for (b, a) in [(1, 3), (2, 2), (4, 4)] {
        v.push(Spec::per_symbol(
            format!("calendar.turn_of_month.{b}.{a}"),
            "calendar",
            format!("Long over the last {b} and first {a} trading days of each month."),
            move || TurnOfMonth {
                before: b,
                after: a,
            },
        ));
    }
    v.push(Spec::per_symbol(
        "calendar.halloween",
        "calendar",
        "'Sell in May and go away': long November-April, cash May-October.",
        || Months(vec![11, 12, 1, 2, 3, 4]),
    ));
    v.push(Spec::per_symbol(
        "calendar.summer_only",
        "calendar",
        "The mirror image of Halloween: long May-October only. A control.",
        || Months(vec![5, 6, 7, 8, 9, 10]),
    ));
    v.push(Spec::per_symbol(
        "calendar.santa_january",
        "calendar",
        "Long only in December and January.",
        || Months(vec![12, 1]),
    ));
    for (wd, name) in [
        (Weekday::Mon, "mon"),
        (Weekday::Tue, "tue"),
        (Weekday::Wed, "wed"),
        (Weekday::Thu, "thu"),
        (Weekday::Fri, "fri"),
    ] {
        v.push(Spec::per_symbol(
            format!("calendar.day_of_week.{name}"),
            "calendar",
            format!("Hold only on {name} (open to next open)."),
            move || DayOfWeek(wd),
        ));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_helpers() {
        let d = NaiveDate::from_ymd_opt(2024, 5, 31).unwrap(); // Friday
        assert_eq!(
            next_weekday(d),
            NaiveDate::from_ymd_opt(2024, 6, 3).unwrap()
        );
        assert_eq!(
            weekdays_left(NaiveDate::from_ymd_opt(2024, 5, 29).unwrap()),
            2
        );
        assert_eq!(
            weekday_of_month(NaiveDate::from_ymd_opt(2024, 6, 3).unwrap()),
            1
        );
        assert_eq!(
            days_in_month(NaiveDate::from_ymd_opt(2024, 2, 10).unwrap()),
            29
        );
    }
}

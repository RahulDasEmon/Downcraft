//! The Scheduler's settings (what the pop-up edits) and the maths that decides when they fire.
//!
//! Times are local wall-clock times. `Manager::tick` asks "did a start / stop time pass since the
//! last tick?", so a time is never missed and never fires twice, whatever the tick rate.
use chrono::{Datelike, Duration, Local, NaiveDate, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};

/// Longest gap (seconds) one tick is allowed to look back over, so a machine that slept for weeks
/// does not replay old times.
const MAX_LOOKBACK: i64 = 8 * 86_400;

#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    /// Start the main queue as soon as DownCraft starts.
    pub on_launch: bool,
    /// Start the main queue at `start_min`.
    pub start_on: bool,
    /// Minutes after local midnight.
    pub start_min: u32,
    /// `true` = every selected weekday, `false` = once, on `once_date`.
    pub daily: bool,
    /// `YYYY-MM-DD`; empty or invalid = not chosen yet.
    pub once_date: String,
    /// Monday first.
    pub days: [bool; 7],
    /// Stop the main queue at `stop_min`.
    pub stop_on: bool,
    pub stop_min: u32,
    /// Open `open_file` when the queue has finished.
    pub open_on: bool,
    pub open_file: String,
}

impl Default for Schedule {
    fn default() -> Self {
        Schedule {
            on_launch: false,
            start_on: false,
            start_min: 23 * 60,
            daily: false,
            once_date: String::new(),
            days: [true; 7],
            stop_on: false,
            stop_min: 7 * 60 + 30,
            open_on: false,
            open_file: String::new(),
        }
    }
}

fn date_of(ts: i64) -> Option<NaiveDate> {
    Local.timestamp_opt(ts, 0).single().map(|t| t.date_naive())
}

fn at(day: NaiveDate, minutes: u32) -> Option<i64> {
    let m = minutes.min(24 * 60 - 1);
    let time = NaiveTime::from_hms_opt(m / 60, m % 60, 0)?;
    Local.from_local_datetime(&day.and_time(time)).earliest().map(|t| t.timestamp())
}

impl Schedule {
    /// The chosen one-time date, if there is a valid one.
    pub fn once_day(&self) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(self.once_date.trim(), "%Y-%m-%d").ok()
    }

    /// Does the schedule run on `day`?
    fn day_ok(&self, day: NaiveDate) -> bool {
        if self.daily {
            self.days[day.weekday().num_days_from_monday() as usize]
        } else {
            self.once_day() == Some(day)
        }
    }

    /// A stop time that is not later than the start time ends the session that began the evening
    /// before, so it belongs to the following morning.
    fn stop_shift(&self) -> i64 {
        i64::from(self.start_on && self.stop_min <= self.start_min)
    }

    /// First moment after `after` at which `minutes` falls on a day the schedule runs
    /// (`shift` days later than that day).
    fn next(&self, after: i64, minutes: u32, shift: i64) -> Option<i64> {
        let first = date_of(after)? - Duration::days(1 + shift);
        (0..=400).find_map(|i| {
            let d = first + Duration::days(i);
            if !self.day_ok(d) {
                return None;
            }
            at(d + Duration::days(shift), minutes).filter(|t| *t > after)
        })
    }

    /// Did `minutes` (on a schedule day, `shift` days later) pass in `(from, to]`?
    fn passed(&self, from: i64, to: i64, minutes: u32, shift: i64) -> bool {
        let from = from.max(to - MAX_LOOKBACK);
        self.next(from, minutes, shift).map_or(false, |t| t <= to)
    }

    /// The queue should start: the start time passed in `(from, to]`.
    pub fn start_due(&self, from: i64, to: i64) -> bool {
        self.start_on && self.passed(from, to, self.start_min, 0)
    }

    /// The queue should stop: the stop time passed in `(from, to]`.
    pub fn stop_due(&self, from: i64, to: i64) -> bool {
        self.stop_on && self.passed(from, to, self.stop_min, self.stop_shift())
    }

    /// When the queue will start next, for display.
    pub fn next_start(&self, now: i64) -> Option<i64> {
        self.start_on.then(|| self.next(now, self.start_min, 0)).flatten()
    }

    /// When the queue will stop next, for display.
    pub fn next_stop(&self, now: i64) -> Option<i64> {
        self.stop_on.then(|| self.next(now, self.stop_min, self.stop_shift())).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(y: i32, m: u32, d: u32, hh: u32, mm: u32) -> i64 {
        let day = NaiveDate::from_ymd_opt(y, m, d).unwrap();
        at(day, hh * 60 + mm).unwrap()
    }

    fn daily(start: (u32, u32)) -> Schedule {
        Schedule { start_on: true, start_min: start.0 * 60 + start.1, daily: true, ..Schedule::default() }
    }

    #[test]
    fn daily_start_fires_once_when_its_minute_passes() {
        let s = daily((23, 0));
        assert!(!s.start_due(ts(2026, 10, 25, 22, 58), ts(2026, 10, 25, 22, 59)));
        assert!(s.start_due(ts(2026, 10, 25, 22, 59), ts(2026, 10, 25, 23, 0)));
        assert!(!s.start_due(ts(2026, 10, 25, 23, 0), ts(2026, 10, 25, 23, 1)));
        assert!(s.start_due(ts(2026, 10, 25, 23, 0), ts(2026, 10, 26, 23, 0)));
    }

    #[test]
    fn a_long_gap_still_catches_the_time_inside_it() {
        let s = daily((9, 30));
        // the machine slept from 22:00 to 10:00 the next day
        assert!(s.start_due(ts(2026, 10, 25, 22, 0), ts(2026, 10, 26, 10, 0)));
    }

    #[test]
    fn only_the_selected_weekdays_run() {
        // 2026-10-25 is a Sunday
        let mut s = daily((8, 0));
        s.days = [true, true, true, true, true, false, false]; // Mon-Fri
        assert!(!s.start_due(ts(2026, 10, 25, 7, 0), ts(2026, 10, 25, 9, 0)));
        assert!(s.start_due(ts(2026, 10, 26, 7, 0), ts(2026, 10, 26, 9, 0)));
    }

    #[test]
    fn one_time_runs_only_on_its_date() {
        let s = Schedule { start_on: true, start_min: 12 * 60, daily: false, once_date: "2026-10-27".into(), ..Schedule::default() };
        assert!(!s.start_due(ts(2026, 10, 26, 0, 0), ts(2026, 10, 26, 23, 0)));
        assert!(s.start_due(ts(2026, 10, 27, 11, 0), ts(2026, 10, 27, 12, 0)));
        assert_eq!(s.next_start(ts(2026, 10, 25, 0, 0)), Some(ts(2026, 10, 27, 12, 0)));
        assert_eq!(s.next_start(ts(2026, 10, 27, 12, 0)), None);
    }

    #[test]
    fn an_overnight_stop_belongs_to_the_next_morning() {
        let mut s = daily((23, 0));
        s.stop_on = true;
        s.stop_min = 7 * 60;
        assert_eq!(s.next_stop(ts(2026, 10, 25, 23, 30)), Some(ts(2026, 10, 26, 7, 0)));
        assert!(s.stop_due(ts(2026, 10, 26, 6, 59), ts(2026, 10, 26, 7, 0)));
    }

    #[test]
    fn a_same_day_stop_stays_on_that_day() {
        let mut s = daily((1, 0));
        s.stop_on = true;
        s.stop_min = 6 * 60;
        assert_eq!(s.next_stop(ts(2026, 10, 25, 2, 0)), Some(ts(2026, 10, 25, 6, 0)));
    }

    #[test]
    fn nothing_fires_when_switched_off_or_without_a_date() {
        let off = Schedule { start_on: false, daily: true, ..Schedule::default() };
        assert!(!off.start_due(0, ts(2030, 1, 1, 0, 0)));
        let no_date = Schedule { start_on: true, daily: false, ..Schedule::default() };
        assert!(!no_date.start_due(ts(2026, 10, 1, 0, 0), ts(2026, 10, 30, 0, 0)));
        assert_eq!(no_date.next_start(ts(2026, 10, 25, 0, 0)), None);
    }

    #[test]
    fn old_settings_files_without_the_field_still_load() {
        let s: Schedule = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Schedule::default());
    }
}

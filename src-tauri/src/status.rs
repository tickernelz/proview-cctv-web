use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Local};

pub fn clock_at(at: DateTime<Local>) -> String {
    at.format("%H:%M").to_string()
}

pub fn label(notify_on: bool, live: Option<&str>, ended: Option<&str>) -> String {
    if !notify_on {
        return "Alarm notifications off".to_string();
    }
    match (live, ended) {
        (Some(at), _) => format!("Alarm notifications on · session ends {at}"),
        (None, Some(at)) => format!("Alarm notifications paused · session ended {at}"),
        (None, None) => "Alarm notifications on · waiting for a sign-in".to_string(),
    }
}

#[derive(Default)]
pub struct Tracker {
    planned: Mutex<Option<DateTime<Local>>>,
    ended: Mutex<Option<String>>,
    live: AtomicBool,
}

impl Tracker {
    pub fn text(
        &self,
        notify_on: bool,
        remaining: Option<Duration>,
        now: DateTime<Local>,
    ) -> String {
        let Some(remaining) = remaining else {
            if self.live.swap(false, Ordering::Relaxed) {
                let planned = self.planned.lock().ok().and_then(|planned| *planned);
                let at = planned.map_or(now, |planned| planned.min(now));
                if let Ok(mut ended) = self.ended.lock() {
                    *ended = Some(clock_at(at));
                }
            }
            let ended = self.ended.lock().ok().and_then(|ended| ended.clone());
            return label(notify_on, None, ended.as_deref());
        };
        let planned = now + chrono::Duration::from_std(remaining).unwrap_or_default();
        if let Ok(mut slot) = self.planned.lock() {
            *slot = Some(planned);
        }
        self.live.store(true, Ordering::Relaxed);
        let clock = clock_at(planned);
        label(notify_on, Some(&clock), None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(2026, 10, 29, hour, minute, 30)
            .single()
            .expect("a local time")
    }

    #[test]
    fn the_clock_is_local_and_two_digits_wide() {
        assert_eq!(clock_at(at(21, 14)), "21:14");
        assert_eq!(clock_at(at(9, 5)), "09:05");
        assert_eq!(clock_at(at(0, 0)), "00:00");
    }

    #[test]
    fn a_live_session_names_the_hour_it_ends() {
        assert_eq!(
            label(true, Some("21:14"), None),
            "Alarm notifications on · session ends 21:14"
        );
        assert_eq!(
            label(true, Some("21:14"), Some("20:02")),
            "Alarm notifications on · session ends 21:14",
            "the live session outranks the moment the previous one ended"
        );
    }

    #[test]
    fn a_lapsed_session_names_the_hour_it_ended() {
        assert_eq!(
            label(true, None, Some("21:14")),
            "Alarm notifications paused · session ended 21:14"
        );
    }

    #[test]
    fn a_switched_off_feature_says_so_before_anything_about_the_session() {
        assert_eq!(
            label(false, Some("21:14"), Some("20:02")),
            "Alarm notifications off"
        );
    }

    #[test]
    fn a_run_that_never_had_a_session_waits_for_one() {
        assert_eq!(
            label(true, None, None),
            "Alarm notifications on · waiting for a sign-in"
        );
    }

    #[test]
    fn a_session_that_runs_to_its_planned_hour_ends_there() {
        let tracker = Tracker::default();
        assert_eq!(
            tracker.text(true, Some(Duration::from_secs(600)), at(21, 0)),
            "Alarm notifications on · session ends 21:10"
        );
        assert_eq!(
            tracker.text(true, None, at(21, 10)),
            "Alarm notifications paused · session ended 21:10"
        );
    }

    #[test]
    fn a_sign_out_ends_the_session_at_the_moment_it_happened() {
        let tracker = Tracker::default();
        assert_eq!(
            tracker.text(true, Some(Duration::from_secs(3600)), at(20, 30)),
            "Alarm notifications on · session ends 21:30"
        );
        assert_eq!(
            tracker.text(true, None, at(20, 35)),
            "Alarm notifications paused · session ended 20:35",
            "an explicit sign-out ends the session now, not at the hour it would have expired"
        );
        assert_eq!(
            tracker.text(true, None, at(22, 0)),
            "Alarm notifications paused · session ended 20:35",
            "the hour it ended stays put"
        );
    }

    #[test]
    fn a_switched_off_feature_keeps_reading_off_after_the_session_goes() {
        let tracker = Tracker::default();
        tracker.text(false, Some(Duration::from_secs(60)), at(20, 30));
        assert_eq!(
            tracker.text(false, None, at(20, 35)),
            "Alarm notifications off"
        );
    }
}

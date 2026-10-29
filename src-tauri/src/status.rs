use std::time::Duration;

use chrono::{DateTime, Local};

pub fn clock_at(at: DateTime<Local>) -> String {
    at.format("%H:%M").to_string()
}

pub fn clock_in(remaining: Option<Duration>) -> Option<String> {
    let remaining = remaining?;
    let delta = chrono::Duration::from_std(remaining).ok()?;
    Some(clock_at(Local::now() + delta))
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
    fn a_remaining_time_becomes_a_clock_label() {
        let label = clock_in(Some(Duration::from_secs(600))).expect("a clock label");
        assert_eq!(label.len(), 5, "{label}");
        assert_eq!(label.as_bytes()[2], b':', "{label}");
        assert_eq!(clock_in(None), None);
    }
}

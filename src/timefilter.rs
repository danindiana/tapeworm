/// Helpers for parsing human-readable time durations and converting them
/// to unix timestamps for use in SQL WHERE clauses.
use anyhow::{anyhow, Result};
use chrono::{Local, TimeZone};

/// Parse a duration string like "30m", "2h", "1d", "1w" into a number of seconds.
pub fn parse_duration_secs(s: &str) -> Result<i64> {
    let s = s.trim();
    if s.is_empty() {
        return Err(anyhow!("empty duration string"));
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let n: i64 = num_str.trim()
        .parse()
        .map_err(|_| anyhow!("invalid duration '{}' — expected e.g. 30m, 2h, 1d, 1w", s))?;
    let secs = match unit {
        "s" => n,
        "m" => n * 60,
        "h" => n * 3600,
        "d" => n * 86_400,
        "w" => n * 7 * 86_400,
        other => return Err(anyhow!("unknown time unit '{}' — use s/m/h/d/w", other)),
    };
    Ok(secs)
}

/// Convert "since N units ago" to a unix timestamp lower bound.
pub fn since_unix(duration_str: &str) -> Result<i64> {
    let secs = parse_duration_secs(duration_str)?;
    Ok(chrono::Utc::now().timestamp() - secs)
}

/// Unix timestamp for the start of today in local time.
pub fn today_start_unix() -> i64 {
    let now = Local::now();
    Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .unwrap()
        .timestamp()
}

// Bring trait methods into scope
use chrono::Datelike;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minutes() {
        assert_eq!(parse_duration_secs("30m").unwrap(), 1800);
    }

    #[test]
    fn parse_hours() {
        assert_eq!(parse_duration_secs("2h").unwrap(), 7200);
    }

    #[test]
    fn parse_days() {
        assert_eq!(parse_duration_secs("1d").unwrap(), 86_400);
    }

    #[test]
    fn parse_weeks() {
        assert_eq!(parse_duration_secs("1w").unwrap(), 604_800);
    }

    #[test]
    fn invalid_unit() {
        assert!(parse_duration_secs("5x").is_err());
    }

    #[test]
    fn empty_string() {
        assert!(parse_duration_secs("").is_err());
    }
}

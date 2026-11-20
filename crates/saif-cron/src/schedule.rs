use chrono::{DateTime, Duration, TimeZone, Utc};
use cron::Schedule;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::OnceLock;
use thiserror::Error;

pub const MIN_GRACE_SECONDS: i64 = 120;
pub const MAX_GRACE_SECONDS: i64 = 7200;
pub const ONESHOT_GRACE_SECONDS: i64 = 120;

#[derive(Debug, Error)]
pub enum ScheduleError {
    #[error("Invalid duration: '{0}'. Use format like '30m', '2h', '1d', or a bare unit like 'hour' (defaults to 1).")]
    InvalidDuration(String),
    #[error("Invalid repeat value {0}: use an integer, 'forever', or 'once'.")]
    InvalidRepeat(String),
    #[error("Invalid schedule '{0}'")]
    InvalidSchedule(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CronSchedule {
    Once {
        #[serde(alias = "at")]
        run_at: String,
        #[serde(default)]
        display: String,
    },
    Interval {
        minutes: i64,
        #[serde(default)]
        display: String,
    },
    Cron {
        expr: String,
        #[serde(default)]
        display: String,
    },
}

#[derive(Debug, Clone)]
pub enum RepeatInput<'a> {
    None,
    Int(i64),
    Str(&'a str),
}

/// Matches `cron/jobs.py::normalize_repeat_value`.
pub fn normalize_repeat_value(input: RepeatInput<'_>) -> Result<Option<i64>, ScheduleError> {
    match input {
        RepeatInput::None => Ok(None),
        RepeatInput::Int(n) => Ok(if n <= 0 { None } else { Some(n) }),
        RepeatInput::Str(raw) => {
            let s = raw.trim().to_ascii_lowercase();
            if matches!(s.as_str(), "forever" | "infinite" | "inf" | "none" | "") {
                return Ok(None);
            }
            if matches!(s.as_str(), "once" | "one" | "1x") {
                return Ok(Some(1));
            }
            match s.parse::<i64>() {
                Ok(n) => Ok(if n <= 0 { None } else { Some(n) }),
                Err(_) => Err(ScheduleError::InvalidRepeat(raw.to_string())),
            }
        }
    }
}

fn duration_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(\d*)\s*(m|min|mins|minute|minutes|h|hr|hrs|hour|hours|d|day|days)$")
            .unwrap_or_else(|_| unreachable!())
    })
}

/// Matches `cron/jobs.py::parse_duration`.
pub fn parse_duration(raw: &str) -> Result<i64, ScheduleError> {
    let s = raw.trim().to_ascii_lowercase();
    let Some(caps) = duration_re().captures(&s) else {
        return Err(ScheduleError::InvalidDuration(s));
    };
    let count: i64 = if caps[1].is_empty() {
        1
    } else {
        caps[1]
            .parse::<i64>()
            .map_err(|_| ScheduleError::InvalidDuration(s.clone()))?
    };
    let mult = match caps[2].chars().next() {
        Some('m') => 1,
        Some('h') => 60,
        Some('d') => 1440,
        _ => return Err(ScheduleError::InvalidDuration(s)),
    };
    Ok(count * mult)
}

fn weekday_to_dow(tok: &str) -> Option<&'static str> {
    match tok {
        "sunday" | "sun" => Some("0"),
        "monday" | "mon" => Some("1"),
        "tuesday" | "tue" | "tues" => Some("2"),
        "wednesday" | "wed" | "weds" => Some("3"),
        "thursday" | "thu" | "thur" | "thurs" => Some("4"),
        "friday" | "fri" => Some("5"),
        "saturday" | "sat" => Some("6"),
        _ => None,
    }
}

fn dayspec_to_dow(tok: &str) -> Option<&'static str> {
    match tok {
        "day" | "daily" | "everyday" => Some("*"),
        "weekday" | "weekdays" => Some("1-5"),
        "weekend" | "weekends" => Some("0,6"),
        _ => None,
    }
}

fn clock_time_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(\d{1,2})(?::(\d{2}))?(am|pm)?$").unwrap_or_else(|_| unreachable!())
    })
}

fn parse_clock_time(text: &str) -> Option<(u32, u32)> {
    let t = text.trim().to_ascii_lowercase().replace(' ', "");
    if t.is_empty() {
        return None;
    }
    if t == "noon" || t == "midday" {
        return Some((12, 0));
    }
    if t == "midnight" {
        return Some((0, 0));
    }
    let caps = clock_time_re().captures(&t)?;
    let mut hour: u32 = caps[1].parse().ok()?;
    let minute: u32 = caps
        .get(2)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0);
    if let Some(mer) = caps.get(3).map(|m| m.as_str()) {
        if !(1..=12).contains(&hour) {
            return None;
        }
        hour = (hour % 12) + if mer == "pm" { 12 } else { 0 };
    }
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

fn natural_every_to_cron(rest: &str) -> Option<String> {
    let lower = rest.to_ascii_lowercase().replace(',', " ");
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    let (dow, idx) = if let Some(spec) = dayspec_to_dow(tokens[0]) {
        (spec.to_string(), 1usize)
    } else {
        let mut days: Vec<&str> = Vec::new();
        let mut stop_idx = tokens.len();
        for (i, &tok) in tokens.iter().enumerate() {
            if tok == "and" {
                continue;
            }
            if let Some(d) = weekday_to_dow(tok) {
                if !days.contains(&d) {
                    days.push(d);
                }
            } else {
                stop_idx = i;
                break;
            }
        }
        if days.is_empty() {
            return None;
        }
        (days.join(","), stop_idx)
    };

    let mut time_tokens = &tokens[idx..];
    if time_tokens.first().copied() == Some("at") {
        time_tokens = &time_tokens[1..];
    }
    if time_tokens.is_empty() {
        return None;
    }
    let (hour, minute) = parse_clock_time(&time_tokens.join(" "))?;
    Some(format!("{minute} {hour} * * {dow}"))
}

fn cron_field_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z\d\*\-,/]+$").unwrap_or_else(|_| unreachable!()))
}

fn map_standard_dow_token(dow: &str) -> String {
    let mut out = String::with_capacity(dow.len() * 2);
    let mut chars = dow.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_ascii_digit() && !chars.peek().is_some_and(|c| c.is_ascii_digit()) {
            let name = match ch {
                '0' | '7' => "SUN",
                '1' => "MON",
                '2' => "TUE",
                '3' => "WED",
                '4' => "THU",
                '5' => "FRI",
                '6' => "SAT",
                _ => {
                    out.push(ch);
                    continue;
                }
            };
            out.push_str(name);
        } else {
            out.push(ch);
        }
    }
    out
}

/// Convert a 5-field standard cron expression to `cron::Schedule`'s 7-field parser (`0 <5-field> *`).
fn parse_5field_cron(expr: &str) -> Result<Schedule, ScheduleError> {
    let parts: Vec<&str> = expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Schedule::from_str(expr)
            .map_err(|_| ScheduleError::InvalidSchedule(expr.to_string()));
    }
    let dow = map_standard_dow_token(parts[4]);
    let normalized = format!(
        "0 {} {} {} {} {} *",
        parts[0], parts[1], parts[2], parts[3], dow
    );
    Schedule::from_str(&normalized).map_err(|_| ScheduleError::InvalidSchedule(expr.to_string()))
}

pub fn parse_schedule(raw: &str) -> Result<CronSchedule, ScheduleError> {
    parse_schedule_at(raw, Utc::now())
}

pub fn parse_schedule_at(raw: &str, now: DateTime<Utc>) -> Result<CronSchedule, ScheduleError> {
    let schedule = raw.trim();
    let lower = schedule.to_ascii_lowercase();

    let is_every = lower.starts_with("every ");
    let rest = if is_every {
        schedule[6..].trim()
    } else {
        lower.as_str()
    };

    if let Some(cron_expr) = natural_every_to_cron(rest) {
        parse_5field_cron(&cron_expr)?;
        return Ok(CronSchedule::Cron {
            expr: cron_expr,
            display: schedule.to_string(),
        });
    }
    if is_every {
        let minutes = parse_duration(rest)
            .map_err(|_| ScheduleError::InvalidSchedule(schedule.to_string()))?;
        return Ok(CronSchedule::Interval {
            minutes,
            display: format!("every {minutes}m"),
        });
    }

    let parts: Vec<&str> = schedule.split_whitespace().collect();
    if parts.len() >= 5 && parts[..5].iter().all(|p| cron_field_re().is_match(p)) {
        parse_5field_cron(schedule)?;
        return Ok(CronSchedule::Cron {
            expr: schedule.to_string(),
            display: schedule.to_string(),
        });
    }

    if schedule.contains('T')
        || (schedule.len() >= 10
            && schedule.as_bytes().get(4) == Some(&b'-')
            && schedule.as_bytes().get(7) == Some(&b'-'))
    {
        if let Ok(dt) = DateTime::parse_from_rfc3339(schedule) {
            let utc = dt.with_timezone(&Utc);
            return Ok(CronSchedule::Once {
                run_at: utc.to_rfc3339(),
                display: format!("once at {}", utc.format("%Y-%m-%d %H:%M")),
            });
        }
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(schedule, "%Y-%m-%dT%H:%M:%S") {
            let utc = Utc.from_utc_datetime(&naive);
            return Ok(CronSchedule::Once {
                run_at: utc.to_rfc3339(),
                display: format!("once at {}", utc.format("%Y-%m-%d %H:%M")),
            });
        }
        return Err(ScheduleError::InvalidSchedule(schedule.to_string()));
    }

    if let Some(dur_str) = lower.strip_prefix("in ") {
        let minutes = parse_duration(dur_str.trim())
            .map_err(|_| ScheduleError::InvalidSchedule(schedule.to_string()))?;
        let run_at = now + Duration::minutes(minutes);
        return Ok(CronSchedule::Once {
            run_at: run_at.to_rfc3339(),
            display: format!("once in {}", dur_str.trim()),
        });
    }

    if let Ok(minutes) = parse_duration(schedule) {
        return Ok(CronSchedule::Interval {
            minutes,
            display: format!("every {minutes}m"),
        });
    }

    Err(ScheduleError::InvalidSchedule(schedule.to_string()))
}

pub fn schedule_cadence_seconds(schedule: &CronSchedule, now: DateTime<Utc>) -> Option<i64> {
    match schedule {
        CronSchedule::Once { .. } => None,
        CronSchedule::Interval { minutes, .. } => (*minutes > 0).then_some(*minutes * 60),
        CronSchedule::Cron { expr, .. } => {
            let parsed = parse_5field_cron(expr).ok()?;
            let mut iter = parsed.after(&now);
            let first = iter.next()?;
            let second = iter.next()?;
            let secs = (second - first).num_seconds();
            (secs > 0).then_some(secs)
        }
    }
}

/// Catch-up window: `clamp(period / 2, 120s, 7200s)` (`_compute_grace_seconds` parity).
pub fn compute_grace_seconds(schedule: &CronSchedule, now: DateTime<Utc>) -> i64 {
    let Some(period) = schedule_cadence_seconds(schedule, now) else {
        return MIN_GRACE_SECONDS;
    };
    (period / 2).clamp(MIN_GRACE_SECONDS, MAX_GRACE_SECONDS)
}

pub fn compute_next_run(
    schedule: &CronSchedule,
    now: DateTime<Utc>,
    last_run_at: Option<&str>,
) -> Option<String> {
    match schedule {
        CronSchedule::Once { run_at, .. } => {
            if last_run_at.is_some() {
                return None;
            }
            let dt = DateTime::parse_from_rfc3339(run_at)
                .ok()?
                .with_timezone(&Utc);
            if (now - dt).num_seconds() <= ONESHOT_GRACE_SECONDS {
                Some(dt.to_rfc3339())
            } else {
                None
            }
        }
        CronSchedule::Interval { minutes, .. } => {
            let base = last_run_at
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or(now);
            Some((base + Duration::minutes(*minutes)).to_rfc3339())
        }
        CronSchedule::Cron { expr, .. } => {
            let base = last_run_at
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or(now);
            let parsed = parse_5field_cron(expr).ok()?;
            parsed.after(&base).next().map(|dt| dt.to_rfc3339())
        }
    }
}

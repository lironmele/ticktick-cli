//! Parsing of user-supplied dates, times, durations, reminders and repeat rules.

use anyhow::{Result, anyhow, bail};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};

#[derive(Debug, Clone, Copy)]
pub struct When {
    pub dt: NaiveDateTime,
    /// False for a bare date (an all-day task).
    pub has_time: bool,
}

pub fn today() -> NaiveDate {
    crate::ctx::now_local().date()
}

fn midnight(d: NaiveDate) -> NaiveDateTime {
    d.and_hms_opt(0, 0, 0).unwrap()
}

fn parse_time(s: &str) -> Option<NaiveTime> {
    let s = s.trim().to_ascii_lowercase();
    let (body, pm, am) = if let Some(b) = s.strip_suffix("pm") {
        (b.trim().to_string(), true, false)
    } else if let Some(b) = s.strip_suffix("am") {
        (b.trim().to_string(), false, true)
    } else {
        (s.clone(), false, false)
    };
    let mut t = NaiveTime::parse_from_str(&body, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(&body, "%H:%M"))
        .ok()
        .or_else(|| {
            let h: u32 = body.parse().ok()?;
            if (pm || am) && h <= 12 {
                NaiveTime::from_hms_opt(h, 0, 0)
            } else {
                None
            }
        })?;
    use chrono::Timelike;
    if pm && t.hour() < 12 {
        t += Duration::hours(12);
    }
    if am && t.hour() == 12 {
        t -= Duration::hours(12);
    }
    Some(t)
}

fn parse_weekday(s: &str) -> Option<Weekday> {
    Some(match s {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "sat" | "saturday" => Weekday::Sat,
        "sun" | "sunday" => Weekday::Sun,
        _ => return None,
    })
}

fn parse_day_word(s: &str) -> Option<NaiveDate> {
    let t = today();
    match s {
        "today" | "tod" => return Some(t),
        "tomorrow" | "tmr" | "tom" => return Some(t + Duration::days(1)),
        "yesterday" => return Some(t - Duration::days(1)),
        _ => {}
    }
    let wd_word = s.strip_prefix("next ").unwrap_or(s);
    if let Some(wd) = parse_weekday(wd_word) {
        let mut d = t + Duration::days(1);
        while d.weekday() != wd {
            d += Duration::days(1);
        }
        return Some(d);
    }
    for fmt in ["%Y-%m-%d", "%Y/%m/%d", "%Y%m%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return Some(d);
        }
    }
    None
}

/// Signed offset like "+3d", "-2w", "+90m", "+1h".
fn parse_offset(s: &str) -> Option<(i64, char)> {
    let (sign, rest) = match s.chars().next()? {
        '+' => (1, &s[1..]),
        '-' => (-1, &s[1..]),
        _ => return None,
    };
    let unit = rest.chars().last()?;
    let n: i64 = rest[..rest.len() - unit.len_utf8()].parse().ok()?;
    Some((sign * n, unit))
}

/// Parse a date expression. Accepted: today, tomorrow, yesterday, now,
/// weekday names ("fri", "next monday"), YYYY-MM-DD, offsets (+3d, -1w, +2h, +30m),
/// each optionally followed by a time ("tomorrow 9:30", "2026-11-01 14:00", "fri 5pm"),
/// and ISO "YYYY-MM-DDTHH:MM[:SS]".
pub fn parse_when(input: &str) -> Result<When> {
    let s = input.trim().to_ascii_lowercase();
    if s.is_empty() {
        bail!("empty date");
    }
    if s == "now" {
        return Ok(When {
            dt: crate::ctx::now_local(),
            has_time: true,
        });
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(&s, fmt) {
            return Ok(When { dt, has_time: true });
        }
    }
    if let Some((n, unit)) = parse_offset(&s) {
        let now = crate::ctx::now_local();
        return Ok(match unit {
            'd' => When {
                dt: midnight(today() + Duration::days(n)),
                has_time: false,
            },
            'w' => When {
                dt: midnight(today() + Duration::weeks(n)),
                has_time: false,
            },
            'h' => When {
                dt: now + Duration::hours(n),
                has_time: true,
            },
            'm' => When {
                dt: now + Duration::minutes(n),
                has_time: true,
            },
            _ => bail!("unknown offset unit in '{input}' (use d, w, h or m)"),
        });
    }
    if let Some(d) = parse_day_word(&s) {
        return Ok(When {
            dt: midnight(d),
            has_time: false,
        });
    }
    // "<day> <time>": try every split point so "next monday 9:00" works.
    let parts: Vec<&str> = s.split_whitespace().collect();
    for i in (1..parts.len()).rev() {
        let day = parts[..i].join(" ");
        let time = parts[i..].join(" ");
        if let (Some(d), Some(t)) = (parse_day_word(&day), parse_time(&time)) {
            return Ok(When {
                dt: d.and_time(t),
                has_time: true,
            });
        }
    }
    if let Some(t) = parse_time(&s) {
        return Ok(When {
            dt: today().and_time(t),
            has_time: true,
        });
    }
    Err(anyhow!(
        "Cannot parse date '{input}'. Use YYYY-MM-DD, 'YYYY-MM-DD HH:MM', today, tomorrow, a weekday, or +Nd/+Nw/+Nh/+Nm"
    ))
}

pub fn parse_date(input: &str) -> Result<NaiveDate> {
    Ok(parse_when(input)?.dt.date())
}

/// Parse a stored TickTick datetime string.
pub fn parse_db_dt(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%m/%d/%Y %H:%M:%S"))
        .ok()
        .filter(|d| d.year() > 1)
}

/// Parse a duration like "25m", "1h30m", "90", "1.5h".
pub fn parse_duration(input: &str) -> Result<Duration> {
    let s = input.trim().to_ascii_lowercase();
    if let Ok(m) = s.parse::<i64>() {
        return Ok(Duration::minutes(m));
    }
    let mut total = 0f64;
    let mut num = String::new();
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let v: f64 = num.parse().map_err(|_| anyhow!("bad duration '{input}'"))?;
        num.clear();
        total += match c {
            'h' => v * 3600.0,
            'm' => v * 60.0,
            's' => v,
            'd' => v * 86400.0,
            _ => bail!("bad duration unit '{c}' in '{input}'"),
        };
    }
    if !num.is_empty() {
        bail!("bad duration '{input}' (missing unit)");
    }
    Ok(Duration::seconds(total as i64))
}

// ---------------------------------------------------------------------------
// Reminders: iCalendar TRIGGER values relative to the task start.
// ---------------------------------------------------------------------------

/// Accepts a raw "TRIGGER:..." value, an offset ("0", "-5m", "-1h", "-2d", "-1h30m"),
/// or for all-day tasks a time of day ("9:00" on the day, "-1d@9:00" the day before).
pub fn parse_reminder(input: &str) -> Result<String> {
    let s = input.trim();
    if s.to_ascii_uppercase().starts_with("TRIGGER:") {
        return Ok(format!("TRIGGER:{}", &s[8..]));
    }
    if s == "0"
        || s == "-0"
        || s.eq_ignore_ascii_case("on-time")
        || s.eq_ignore_ascii_case("ontime")
    {
        return Ok("TRIGGER:PT0S".into());
    }
    // all-day form: [-Nd@]HH:MM
    let (days, time) = match s.split_once('@') {
        Some((d, t)) => (Some(d), Some(t)),
        None if s.contains(':') => (None, Some(s)),
        None => (None, None),
    };
    if let Some(t) = time {
        let t = parse_time(t).ok_or_else(|| anyhow!("bad reminder time '{input}'"))?;
        use chrono::Timelike;
        let days_before: i64 = match days {
            Some(d) => {
                let d = d.trim().trim_start_matches('-').trim_end_matches('d');
                d.parse().map_err(|_| anyhow!("bad reminder '{input}'"))?
            }
            None => 0,
        };
        // Day-before reminders count back from midnight: -P1DT15H means 09:00 the previous day.
        if days_before == 0 {
            return Ok(format!("TRIGGER:P0DT{}H{}M0S", t.hour(), t.minute()));
        }
        let back_secs = days_before * 86400 - i64::from(t.num_seconds_from_midnight());
        let d = back_secs / 86400;
        let rem = back_secs % 86400;
        return Ok(format!(
            "TRIGGER:-P{}DT{}H{}M0S",
            d,
            rem / 3600,
            (rem % 3600) / 60
        ));
    }
    let neg = s.starts_with('-');
    let body = s.trim_start_matches(['-', '+']);
    let dur = parse_duration(body)?;
    let mut secs = dur.num_seconds();
    let sign = if neg && secs != 0 { "-" } else { "" };
    let d = secs / 86400;
    secs %= 86400;
    let (h, m, sec) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    let mut out = format!("TRIGGER:{sign}P");
    if d > 0 {
        out.push_str(&format!("{d}D"));
    }
    if h > 0 || m > 0 || sec > 0 || d == 0 {
        out.push('T');
        if h > 0 {
            out.push_str(&format!("{h}H"));
        }
        if m > 0 {
            out.push_str(&format!("{m}M"));
        }
        if sec > 0 || (h == 0 && m == 0) {
            out.push_str(&format!("{sec}S"));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Repeat rules
// ---------------------------------------------------------------------------

/// Accepts a raw RRULE or a shortcut: daily, weekly, monthly, yearly, weekdays,
/// weekends, "every 2 days", "every 3 weeks", "every mon,wed,fri".
pub fn parse_repeat(input: &str) -> Result<String> {
    let s = input.trim();
    let up = s.to_ascii_uppercase();
    if up.starts_with("RRULE:") {
        return Ok(s.to_string());
    }
    if up.starts_with("FREQ=") {
        return Ok(format!("RRULE:{s}"));
    }
    let l = s.to_ascii_lowercase();
    let rule = match l.as_str() {
        "daily" | "every day" => "FREQ=DAILY;INTERVAL=1".to_string(),
        "weekly" | "every week" => "FREQ=WEEKLY;INTERVAL=1".to_string(),
        "monthly" | "every month" => "FREQ=MONTHLY;INTERVAL=1".to_string(),
        "yearly" | "annually" | "every year" => "FREQ=YEARLY;INTERVAL=1".to_string(),
        "weekdays" | "every weekday" => "FREQ=WEEKLY;INTERVAL=1;BYDAY=MO,TU,WE,TH,FR".to_string(),
        "weekends" | "every weekend" => "FREQ=WEEKLY;INTERVAL=1;BYDAY=SA,SU".to_string(),
        _ => {
            let rest = l
                .strip_prefix("every ")
                .ok_or_else(|| anyhow!("Cannot parse repeat '{input}'. Use daily/weekly/monthly/yearly/weekdays, 'every 2 weeks', 'every mon,thu', or an RRULE"))?;
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() == 2
                && let Ok(n) = parts[0].parse::<u32>()
            {
                let freq = match parts[1].trim_end_matches('s') {
                    "day" => "DAILY",
                    "week" => "WEEKLY",
                    "month" => "MONTHLY",
                    "year" => "YEARLY",
                    other => bail!("unknown repeat unit '{other}'"),
                };
                return Ok(format!("RRULE:FREQ={freq};INTERVAL={n}"));
            }
            let days: Result<Vec<&str>> = rest
                .split([',', ' '])
                .filter(|d| !d.is_empty())
                .map(|d| {
                    parse_weekday(d)
                        .map(byday_code)
                        .ok_or_else(|| anyhow!("unknown weekday '{d}' in repeat"))
                })
                .collect();
            format!("FREQ=WEEKLY;INTERVAL=1;BYDAY={}", days?.join(","))
        }
    };
    Ok(format!("RRULE:{rule}"))
}

fn byday_code(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

fn weekday_from_code(c: &str) -> Option<Weekday> {
    // BYDAY entries can carry an ordinal ("1MO", "-1FR"); keep the day part.
    let code = c.trim_start_matches(|ch: char| ch == '-' || ch == '+' || ch.is_ascii_digit());
    Some(match code {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

pub struct Rrule {
    pub freq: String,
    pub interval: i64,
    pub byday: Vec<Weekday>,
    pub bymonthday: Option<i64>,
    pub count: Option<i64>,
    pub until: Option<NaiveDate>,
    parts: Vec<(String, String)>,
}

impl Rrule {
    pub fn parse(rule: &str) -> Result<Self> {
        let body = rule.trim();
        let body = body
            .strip_prefix("RRULE:")
            .or_else(|| body.strip_prefix("rrule:"))
            .unwrap_or(body);
        // ERULE:/other prefixes are not supported.
        let parts: Vec<(String, String)> = body
            .split(';')
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.to_ascii_uppercase(), v.to_string()))
            .collect();
        let get = |k: &str| parts.iter().find(|(pk, _)| pk == k).map(|(_, v)| v.clone());
        let freq = get("FREQ").ok_or_else(|| anyhow!("repeat rule has no FREQ: {rule}"))?;
        let byday = get("BYDAY")
            .map(|v| v.split(',').filter_map(weekday_from_code).collect())
            .unwrap_or_default();
        let until = get("UNTIL")
            .and_then(|u| NaiveDate::parse_from_str(&u[..u.len().min(8)], "%Y%m%d").ok());
        Ok(Self {
            freq: freq.to_ascii_uppercase(),
            interval: get("INTERVAL")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1)
                .max(1),
            byday,
            bymonthday: get("BYMONTHDAY").and_then(|v| v.split(',').next()?.parse().ok()),
            count: get("COUNT").and_then(|v| v.parse().ok()),
            until,
            parts,
        })
    }

    /// Unsupported constructs make the CLI refuse rather than guess.
    pub fn check_supported(&self) -> Result<()> {
        for (k, _) in &self.parts {
            if ![
                "FREQ",
                "INTERVAL",
                "BYDAY",
                "BYMONTHDAY",
                "COUNT",
                "UNTIL",
                "WKST",
                "TT_SKIP",
            ]
            .contains(&k.as_str())
            {
                bail!("repeat rule part {k} is not supported by the CLI");
            }
        }
        if !["DAILY", "WEEKLY", "MONTHLY", "YEARLY"].contains(&self.freq.as_str()) {
            bail!("repeat frequency {} is not supported by the CLI", self.freq);
        }
        if self.freq == "MONTHLY"
            && self
                .parts
                .iter()
                .any(|(k, v)| k == "BYDAY" && !v.is_empty())
        {
            bail!("monthly repeats by weekday (e.g. 'first Monday') are not supported by the CLI");
        }
        Ok(())
    }

    /// The first occurrence strictly after `after`.
    pub fn next_after(&self, anchor: NaiveDate, after: NaiveDate) -> Option<NaiveDate> {
        let mut candidate = anchor;
        for _ in 0..5000 {
            candidate = self.step(candidate, anchor)?;
            if candidate > after {
                if let Some(u) = self.until
                    && candidate > u
                {
                    return None;
                }
                return Some(candidate);
            }
        }
        None
    }

    fn step(&self, d: NaiveDate, anchor: NaiveDate) -> Option<NaiveDate> {
        match self.freq.as_str() {
            "DAILY" => Some(d + Duration::days(self.interval)),
            "WEEKLY" => {
                if self.byday.is_empty() {
                    return Some(d + Duration::weeks(self.interval));
                }
                let week_start =
                    |x: NaiveDate| x - Duration::days(x.weekday().num_days_from_monday() as i64);
                let anchor_week = week_start(anchor);
                let mut x = d + Duration::days(1);
                for _ in 0..(7 * self.interval + 7) {
                    let weeks = (week_start(x) - anchor_week).num_weeks();
                    if weeks % self.interval == 0 && self.byday.contains(&x.weekday()) {
                        return Some(x);
                    }
                    x += Duration::days(1);
                }
                None
            }
            "MONTHLY" => {
                let day = self.bymonthday.unwrap_or(anchor.day() as i64);
                let mut months = d.year() as i64 * 12 + d.month0() as i64;
                for _ in 0..48 {
                    months += self.interval;
                    let (y, m) = ((months / 12) as i32, (months % 12) as u32 + 1);
                    let last = last_day_of_month(y, m);
                    let dd = if day < 0 { last as i64 + day + 1 } else { day };
                    if dd >= 1 && dd <= last as i64 {
                        return NaiveDate::from_ymd_opt(y, m, dd as u32);
                    }
                }
                None
            }
            "YEARLY" => {
                let mut y = d.year();
                for _ in 0..8 {
                    y += self.interval as i32;
                    if let Some(x) = NaiveDate::from_ymd_opt(y, anchor.month(), anchor.day()) {
                        return Some(x);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// The rule string after one occurrence is consumed (COUNT decremented).
    pub fn after_one_occurrence(rule: &str) -> String {
        let mut out = Vec::new();
        for part in rule.trim_start_matches("RRULE:").split(';') {
            if let Some(v) = part.strip_prefix("COUNT=")
                && let Ok(n) = v.parse::<i64>()
            {
                out.push(format!("COUNT={}", (n - 1).max(0)));
                continue;
            }
            out.push(part.to_string());
        }
        format!("RRULE:{}", out.join(";"))
    }
}

fn last_day_of_month(y: i32, m: u32) -> u32 {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    (NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - Duration::days(1)).day()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reminders() {
        assert_eq!(parse_reminder("0").unwrap(), "TRIGGER:PT0S");
        assert_eq!(parse_reminder("-5m").unwrap(), "TRIGGER:-PT5M");
        assert_eq!(parse_reminder("-1h30m").unwrap(), "TRIGGER:-PT1H30M");
        assert_eq!(parse_reminder("-1d").unwrap(), "TRIGGER:-P1D");
        assert_eq!(parse_reminder("9:00").unwrap(), "TRIGGER:P0DT9H0M0S");
        assert_eq!(parse_reminder("-1d@9:00").unwrap(), "TRIGGER:-P0DT15H0M0S");
        assert_eq!(parse_reminder("-2d@9:00").unwrap(), "TRIGGER:-P1DT15H0M0S");
    }

    #[test]
    fn repeats() {
        assert_eq!(
            parse_repeat("daily").unwrap(),
            "RRULE:FREQ=DAILY;INTERVAL=1"
        );
        assert_eq!(
            parse_repeat("every 2 weeks").unwrap(),
            "RRULE:FREQ=WEEKLY;INTERVAL=2"
        );
        assert_eq!(
            parse_repeat("every mon,fri").unwrap(),
            "RRULE:FREQ=WEEKLY;INTERVAL=1;BYDAY=MO,FR"
        );
    }

    #[test]
    fn rrule_next() {
        let d = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let r = Rrule::parse("RRULE:FREQ=WEEKLY;INTERVAL=1;BYDAY=MO,WE,FR").unwrap();
        assert_eq!(
            r.next_after(d("2026-10-19"), d("2026-10-19")),
            Some(d("2026-10-21"))
        );
        assert_eq!(
            r.next_after(d("2026-10-19"), d("2026-10-23")),
            Some(d("2026-10-26"))
        );
        let r = Rrule::parse("RRULE:FREQ=MONTHLY;INTERVAL=1").unwrap();
        assert_eq!(
            r.next_after(d("2026-01-31"), d("2026-01-31")),
            Some(d("2026-03-31"))
        );
        let r = Rrule::parse("RRULE:FREQ=DAILY;INTERVAL=3").unwrap();
        assert_eq!(
            r.next_after(d("2026-10-01"), d("2026-10-05")),
            Some(d("2026-10-07"))
        );
        let r = Rrule::parse("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=TU").unwrap();
        assert_eq!(
            r.next_after(d("2026-10-20"), d("2026-10-20")),
            Some(d("2026-11-03"))
        );
        assert_eq!(
            Rrule::after_one_occurrence("RRULE:FREQ=DAILY;COUNT=3"),
            "RRULE:FREQ=DAILY;COUNT=2"
        );
    }

    #[test]
    fn whens() {
        let w = parse_when("2026-11-01 14:30").unwrap();
        assert!(w.has_time);
        assert_eq!(w.dt.to_string(), "2026-11-01 14:30:00");
        let w = parse_when("2026-11-01").unwrap();
        assert!(!w.has_time);
        assert!(parse_when("fri 5pm").unwrap().has_time);
        assert!(parse_when("next monday 9:00").unwrap().has_time);
        assert!(parse_when("garbage").is_err());
    }
}

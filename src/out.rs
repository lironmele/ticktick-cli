//! Output helpers: JSON for agents (--json), tables for humans.

use serde_json::Value;
use tabled::builder::Builder;
use tabled::settings::{Alignment, Style};

pub fn json(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
    );
}

pub fn table(headers: &[&str], rows: Vec<Vec<String>>, empty: &str) {
    if rows.is_empty() {
        println!("{empty}");
        return;
    }
    let mut b = Builder::default();
    b.push_record(headers.iter().map(|h| h.to_string()));
    for r in rows {
        b.push_record(r);
    }
    let mut t = b.build();
    t.with(Style::psql());
    t.with(Alignment::left());
    println!("{t}");
}

pub fn s(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(s).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// Short date for tables: "2026-10-20" or "2026-10-20 14:30".
pub fn short_dt(v: &Value, all_day: bool) -> String {
    let raw = s(v);
    if raw.is_empty() {
        return raw;
    }
    if all_day || raw.ends_with("00:00:00") {
        raw.chars().take(10).collect()
    } else {
        raw.chars().take(16).collect()
    }
}

/// Print the result of a mutation.
pub fn done(as_json: bool, v: Value, message: &str) {
    if as_json {
        json(&v);
    } else {
        println!("{message}");
    }
}

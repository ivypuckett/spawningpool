//! The CLI's structured-log sink (docs/workflow-logging.md).
//!
//! Logging is always on: each invocation opens `logs/<datestamp>-<root>.ndjson`
//! in the current directory and returns a [`spawningpool::LogSink`] closure that
//! stamps every event the library emits with the two universal fields it owns —
//! `ts` (RFC 3339, millisecond precision, at emit time) and `run` (8 hex chars,
//! fixed for the invocation) — and writes one NDJSON line. Timestamps and the
//! run id are computed with std only; no date or random crate is pulled in.

use std::cell::RefCell;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// Open this run's log file and return a sink that records each event to it.
/// `root` is the root workflow name (or the specialist name for a bare
/// `run specialist`), used both as the `wf`/identity anchor and in the filename.
pub(crate) fn open_sink(root: &str) -> Result<impl Fn(Value), String> {
    let now = SystemTime::now();
    let run = run_id(now);
    let path = log_path(now, root);
    std::fs::create_dir_all("logs").map_err(|e| format!("can't create logs/ directory: {e}"))?;
    let file = File::create(&path).map_err(|e| format!("can't open log file {path}: {e}"))?;
    let writer = RefCell::new(BufWriter::new(file));

    Ok(move |event: Value| {
        let line = enrich(&run, SystemTime::now(), event);
        // Logging must never take down a run: a write error is dropped. Flush per
        // line so the log stays current even if the process is killed mid-run.
        let mut w = writer.borrow_mut();
        if serde_json::to_writer(&mut *w, &line).is_ok() {
            let _ = w.write_all(b"\n");
            let _ = w.flush();
        }
    })
}

/// Inject the sink-owned universal fields (`ts`, `run`) into a library event.
/// Key order is not significant (see docs/workflow-logging.md).
fn enrich(run: &str, now: SystemTime, mut event: Value) -> Value {
    if let Value::Object(map) = &mut event {
        map.insert("ts".to_string(), Value::String(rfc3339_millis(now)));
        map.insert("run".to_string(), Value::String(run.to_string()));
    }
    event
}

/// `logs/<datestamp>-<root>.ndjson`, with `root` reduced to filesystem-safe
/// characters and `<datestamp>` a compact UTC stamp (`YYYYMMDDThhmmssZ`).
fn log_path(now: SystemTime, root: &str) -> String {
    format!("logs/{}-{}.ndjson", datestamp(now), sanitize(root))
}

/// Map anything outside `[A-Za-z0-9._-]` to `_` so a name is a safe filename.
fn sanitize(root: &str) -> String {
    let cleaned: String = root
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "run".to_string()
    } else {
        cleaned
    }
}

/// 8 hex chars derived from the wall clock and pid — unique enough to tie one
/// invocation's events together (it isn't a security token).
fn run_id(now: SystemTime) -> String {
    let nanos = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mixed = (nanos as u64) ^ ((nanos >> 64) as u64) ^ (u64::from(std::process::id()) << 32);
    format!("{:08x}", (mixed ^ (mixed >> 32)) as u32)
}

/// RFC 3339 with millisecond precision in UTC, e.g. `2026-06-19T14:23:01.042Z`.
fn rfc3339_millis(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s, ms) = civil(t);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{ms:03}Z")
}

/// Compact UTC stamp for filenames, e.g. `20260619T142301Z`.
fn datestamp(t: SystemTime) -> String {
    let (y, mo, d, h, mi, s, _) = civil(t);
    format!("{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}Z")
}

/// Break a [`SystemTime`] into its UTC civil parts: (year, month, day, hour,
/// minute, second, millisecond). Times before the Unix epoch clamp to the epoch.
fn civil(t: SystemTime) -> (i64, u32, u32, u32, u32, u32, u32) {
    let dur = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs() as i64;
    let ms = dur.subsec_millis();
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, mo, d) = civil_from_days(days);
    let h = (sod / 3600) as u32;
    let mi = ((sod % 3600) / 60) as u32;
    let s = (sod % 60) as u32;
    (y, mo, d, h, mi, s, ms)
}

/// Convert a count of days since the Unix epoch to a `(year, month, day)` civil
/// date — Howard Hinnant's `civil_from_days` (proleptic Gregorian).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64, millis: u32) -> SystemTime {
        UNIX_EPOCH + std::time::Duration::new(secs, millis * 1_000_000)
    }

    #[test]
    fn rfc3339_formats_the_epoch() {
        assert_eq!(rfc3339_millis(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn rfc3339_carries_millis_and_time_of_day() {
        // 1 second + 500 ms past the epoch.
        assert_eq!(rfc3339_millis(at(1, 500)), "1970-01-01T00:00:01.500Z");
    }

    #[test]
    fn rfc3339_handles_a_leap_day() {
        // 1582934400 = 2020-02-29T00:00:00Z.
        assert_eq!(
            rfc3339_millis(at(1_582_934_400, 0)),
            "2020-02-29T00:00:00.000Z"
        );
    }

    #[test]
    fn datestamp_is_filesystem_safe_and_compact() {
        assert_eq!(datestamp(at(1_582_934_400, 0)), "20200229T000000Z");
    }

    #[test]
    fn run_id_is_eight_hex_chars() {
        let id = run_id(SystemTime::now());
        assert_eq!(id.len(), 8);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn enrich_injects_ts_and_run_and_keeps_the_event() {
        let event = serde_json::json!({ "event": "workflow.start", "wf": "demo" });
        let out = enrich("f3a9b21c", at(1_582_934_400, 42), event);
        assert_eq!(out["run"], "f3a9b21c");
        assert_eq!(out["ts"], "2020-02-29T00:00:00.042Z");
        assert_eq!(out["event"], "workflow.start");
        assert_eq!(out["wf"], "demo");
    }

    #[test]
    fn log_path_combines_datestamp_and_sanitized_root() {
        assert_eq!(
            log_path(at(1_582_934_400, 0), "my/weird name"),
            "logs/20200229T000000Z-my_weird_name.ndjson"
        );
    }
}

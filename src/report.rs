//! A report of all checks: text for people, JSON for Olympos, and what changed since last time.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};

use crate::checks::{Check, Status};

#[derive(Serialize)]
pub struct Report {
    pub version: &'static str,
    pub time: String,
    pub status: Status,
    pub counts: BTreeMap<&'static str, usize>,
    pub checks: Vec<Check>,
}

pub fn iso(secs: u64) -> String {
    // civil-from-days (Howard Hinnant), UTC
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Report {
    pub fn new(checks: Vec<Check>, now: u64) -> Report {
        let mut counts: BTreeMap<&'static str, usize> = ["ok", "unknown", "warn", "bad"].into_iter().map(|k| (k, 0)).collect();
        for c in &checks {
            *counts.entry(c.status.as_str()).or_default() += 1;
        }
        let status = checks.iter().map(|c| c.status).filter(|s| *s != Status::Unknown).max().unwrap_or(Status::Ok);
        Report { version: crate::VERSION, time: iso(now), status, counts, checks }
    }

    pub fn text(&self, color: bool) -> String {
        let paint = |s: Status, t: &str| {
            if !color {
                return t.to_string();
            }
            let code = match s {
                Status::Ok => "32",
                Status::Unknown => "2",
                Status::Warn => "33",
                Status::Bad => "1;31",
            };
            format!("\x1b[{code}m{t}\x1b[0m")
        };
        let mut out = format!("Hestia {} · {}\n\n", self.version, self.time);
        for c in &self.checks {
            out += &format!("{}  {:<18} {}\n", paint(c.status, &format!("{:<7}", c.status.as_str().to_uppercase())), c.title, c.summary);
            if c.status != Status::Ok {
                for d in &c.details {
                    out += &format!("{:<28}{d}\n", "");
                }
            }
        }
        let n = |k: &str| self.counts.get(k).copied().unwrap_or(0);
        out += &format!("\n{} ok · {} warn · {} bad · {} unknown\n", n("ok"), n("warn"), n("bad"), n("unknown"));
        out
    }

    pub fn json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// What got worse or recovered since the last run (by check id). New checks count as "was ok".
pub fn changes(previous: &BTreeMap<String, Status>, report: &Report) -> Vec<(Check, Status)> {
    report
        .checks
        .iter()
        .filter_map(|c| {
            let before = previous.get(&c.id).copied().unwrap_or(Status::Ok);
            let worse = c.status > before && matches!(c.status, Status::Warn | Status::Bad);
            let recovered = c.status == Status::Ok && matches!(before, Status::Warn | Status::Bad);
            (worse || recovered).then(|| (c.clone(), before))
        })
        .collect()
}

pub fn load_previous(path: &Path) -> BTreeMap<String, Status> {
    let Ok(text) = std::fs::read_to_string(path) else { return BTreeMap::new() };
    let Ok(Value::Object(m)) = serde_json::from_str::<Value>(&text) else { return BTreeMap::new() };
    m.into_iter().filter_map(|(k, v)| Some((k, Status::parse(v.as_str()?)?))).collect()
}

pub fn previous_json(report: &Report) -> Value {
    Value::Object(report.checks.iter().map(|c| (c.id.clone(), json!(c.status.as_str()))).collect())
}

/// Write owner-only, atomically (status files name disks and services).
pub fn save(path: &Path, value: &Value) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(serde_json::to_string_pretty(value).unwrap_or_default().as_bytes())?;
    f.sync_all()?;
    std::fs::rename(tmp, path)
}

//! A small history (at most one sample an hour, 90 days) to see trends a single look cannot:
//! a disk that will be full in three weeks, a battery that lost 5 % in two months.
//!
//! ~/.local/state/hestia/history.jsonl, one JSON object per line:
//!   {"t": 1795500000, "disks": {"/": [total, avail]}, "battery": 96.4}

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::checks::{self, human_bytes, Check, Status, Sys};

pub const EVERY: u64 = 3600; // at most one sample an hour
pub const KEEP: u64 = 90 * 86400;
pub const WINDOW: u64 = 14 * 86400; // the trend looks at the last two weeks
pub const MIN_SPAN: u64 = 2 * 86400; // ... and needs at least two days of them
const STEADY: f64 = 50.0 * 1024.0 * 1024.0; // under 50 MiB a day a disk counts as steady

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub t: u64,
    #[serde(default)]
    pub disks: BTreeMap<String, (u64, u64)>, // mount -> (total, avail)
    #[serde(default)]
    pub battery: Option<f64>,
}

pub fn sample(sys: &Sys, now: u64) -> Sample {
    let disks = checks::mounts(sys).unwrap_or_default().into_iter().map(|m| (m.name, (m.total, m.avail))).collect();
    Sample { t: now, disks, battery: checks::battery_health(sys).map(|h| (h * 10.0).round() / 10.0) }
}

/// Every readable sample, oldest first (a damaged line is skipped, not fatal).
pub fn load(path: &Path) -> Vec<Sample> {
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut v: Vec<Sample> = text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
    v.sort_by_key(|s| s.t);
    v
}

/// Add `s` unless the last sample is younger than an hour; drop samples older than 90 days.
/// Returns the samples after the update.
pub fn record(path: &Path, s: Sample) -> std::io::Result<Vec<Sample>> {
    let mut all = load(path);
    if all.last().is_some_and(|last| s.t < last.t + EVERY) {
        return Ok(all);
    }
    if let Some(dir) = path.parent() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    }
    let old = all.first().is_some_and(|f| f.t + KEEP + 86400 < s.t);
    all.push(s);
    if old {
        // rewrite without the oldest samples (only once a day, not on every run)
        let newest = all.last().map(|l| l.t).unwrap_or(0);
        all.retain(|x| x.t + KEEP >= newest);
        let tmp = path.with_extension("tmp");
        let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        for x in &all {
            writeln!(f, "{}", serde_json::to_string(x).unwrap_or_default())?;
        }
        f.sync_all()?;
        fs::rename(tmp, path)?;
    } else {
        let mut f = OpenOptions::new().append(true).create(true).mode(0o600).open(path)?;
        writeln!(f, "{}", serde_json::to_string(all.last().unwrap()).unwrap_or_default())?;
    }
    Ok(all)
}

/// Least-squares slope of (t, y) in y per second.
fn slope(points: &[(f64, f64)]) -> Option<f64> {
    let n = points.len() as f64;
    if points.len() < 2 {
        return None;
    }
    let (mx, my) = (points.iter().map(|p| p.0).sum::<f64>() / n, points.iter().map(|p| p.1).sum::<f64>() / n);
    let (mut num, mut den) = (0.0, 0.0);
    for (x, y) in points {
        num += (x - mx) * (y - my);
        den += (x - mx) * (x - mx);
    }
    (den > 0.0).then(|| num / den)
}

/// "Disk trend": how fast each disk fills, and when the fastest would be full.
pub fn disk_trend(samples: &[Sample], now: u64) -> Check {
    let recent: Vec<&Sample> = samples.iter().filter(|s| s.t + WINDOW >= now).collect();
    let span = match (recent.first(), recent.last()) {
        (Some(a), Some(b)) => b.t.saturating_sub(a.t),
        _ => 0,
    };
    if span < MIN_SPAN {
        let days = span as f64 / 86400.0;
        return Check::new("disk-trend", "Disk trend", Status::Unknown,
                          format!("collecting history: {days:.1} of 2 days so far"));
    }
    let last = recent.last().unwrap();
    let (mut worst, mut details, mut headline): (Status, Vec<String>, Option<(f64, String)>) = (Status::Ok, vec![], None);
    for (mount, (_, avail)) in &last.disks {
        let pts: Vec<(f64, f64)> = recent
            .iter()
            .filter_map(|s| s.disks.get(mount).map(|(total, avail)| (s.t as f64, total.saturating_sub(*avail) as f64)))
            .collect();
        let Some(per_sec) = slope(&pts) else { continue };
        let per_day = per_sec * 86400.0;
        if per_day < STEADY {
            details.push(format!("{mount}: steady"));
            continue;
        }
        let days_left = *avail as f64 / per_day;
        let st = if days_left < 7.0 { Status::Bad } else if days_left < 30.0 { Status::Warn } else { Status::Ok };
        worst = worst.max(st);
        details.push(format!("{mount}: +{}/day, full in about {} days", human_bytes(per_day as u64), days_left.round()));
        if headline.as_ref().is_none_or(|(d, _)| days_left < *d) {
            headline = Some((days_left, format!("{mount} fills in about {} days (+{}/day over the last {} days)",
                                                days_left.round(), human_bytes(per_day as u64), span / 86400)));
        }
    }
    let summary = match (&headline, worst) {
        (Some((_, h)), Status::Warn | Status::Bad) => h.clone(),
        (Some((d, _)), _) => format!("no disk will be full within a month (the fastest in about {} days)", d.round()),
        (None, _) => format!("disks steady over the last {} days", span / 86400),
    };
    Check::new("disk-trend", "Disk trend", worst, summary).with(details)
}

/// Add "was 98% on 2026-09-25" to the battery check when the history goes back a month or more.
pub fn battery_note(checks: &mut [Check], samples: &[Sample], now: u64) {
    let Some(old) = samples.iter().find(|s| s.battery.is_some() && s.t + 30 * 86400 <= now) else { return };
    let Some(now_h) = samples.iter().rev().find_map(|s| s.battery) else { return };
    let then = old.battery.unwrap_or(now_h);
    if let Some(c) = checks.iter_mut().find(|c| c.id == "battery") {
        let days = (now - old.t) / 86400;
        c.details.push(format!("{days} days ago it held {then:.0}%, now {now_h:.0}% ({:+.1} points)", now_h - then));
    }
}

/// Record a sample of this machine and add the trend checks (after "Disk errors").
pub fn apply(sys: &Sys, path: &Path, now: u64, checks: &mut Vec<Check>) {
    let samples = record(path, sample(sys, now)).unwrap_or_else(|_| load(path));
    let at = checks.iter().position(|c| c.id == "disk-errors").map(|i| i + 1).unwrap_or(checks.len());
    checks.insert(at, disk_trend(&samples, now));
    battery_note(checks, &samples, now);
}

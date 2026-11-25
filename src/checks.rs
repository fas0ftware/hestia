//! The health checks. Every reader takes a root directory (normally "/") so the tests can
//! build a fake /sys and /proc; nothing here needs root or changes the system.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Unknown,
    Warn,
    Bad,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Unknown => "unknown",
            Status::Warn => "warn",
            Status::Bad => "bad",
        }
    }
    pub fn parse(s: &str) -> Option<Status> {
        Some(match s {
            "ok" => Status::Ok,
            "unknown" => Status::Unknown,
            "warn" => Status::Warn,
            "bad" => Status::Bad,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub id: String,
    pub title: String,
    pub status: Status,
    pub summary: String,
    pub details: Vec<String>,
}

impl Check {
    pub fn new(id: &str, title: &str, status: Status, summary: impl Into<String>) -> Check {
        Check { id: id.into(), title: title.into(), status, summary: summary.into(), details: vec![] }
    }
    pub fn with(mut self, details: Vec<String>) -> Check {
        self.details = details;
        self
    }
}

/// Thresholds; the defaults suit a laptop.
#[derive(Clone, Debug)]
pub struct Limits {
    pub disk_warn: f64,
    pub disk_bad: f64,
    pub temp_warn: f64,
    pub temp_bad: f64,
    pub wear_warn: f64,
    pub wear_bad: f64,
    pub mem_warn: f64, // percent of memory still available
    pub mem_bad: f64,
    pub load_warn: f64, // 15-minute load per CPU
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            disk_warn: 85.0,
            disk_bad: 95.0,
            temp_warn: 85.0,
            temp_bad: 95.0,
            wear_warn: 30.0,
            wear_bad: 50.0,
            mem_warn: 10.0,
            mem_bad: 5.0,
            load_warn: 1.5,
        }
    }
}

/// statvfs(mountpoint) -> (total bytes, available bytes, inodes, free inodes)
pub type StatFn = Box<dyn Fn(&str) -> Option<(u64, u64, u64, u64)>>;
/// systemctl arguments -> stdout (None if systemctl could not run)
pub type SystemctlFn = Box<dyn Fn(&[&str]) -> Option<String>>;

/// Files and commands the checks read; swapped out in tests.
pub struct Sys {
    pub root: PathBuf,
    pub statvfs: StatFn,
    pub systemctl: SystemctlFn,
}

impl Sys {
    pub fn real() -> Sys {
        Sys { root: PathBuf::from("/"), statvfs: Box::new(real_statvfs), systemctl: Box::new(real_systemctl) }
    }
    fn path(&self, p: &str) -> PathBuf {
        self.root.join(p.trim_start_matches('/'))
    }
    fn read(&self, p: impl AsRef<Path>) -> Option<String> {
        fs::read_to_string(p).ok().map(|s| s.trim().to_string())
    }
    fn num(&self, p: impl AsRef<Path>) -> Option<f64> {
        self.read(p)?.parse().ok()
    }
}

fn real_statvfs(path: &str) -> Option<(u64, u64, u64, u64)> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c is a valid NUL-terminated path and st a properly sized, writable struct.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let frsize = st.f_frsize as u64;
    Some((st.f_blocks as u64 * frsize, st.f_bavail as u64 * frsize, st.f_files as u64, st.f_ffree as u64))
}

fn real_systemctl(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("systemctl").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Text from the kernel or a device (a sensor label, a unit description): printable ASCII, capped.
pub fn clean(s: &str, limit: usize) -> String {
    s.chars().map(|c| if (' '..='~').contains(&c) { c } else { '?' }).take(limit).collect()
}

fn sorted_dir(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(p).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    v.sort();
    v
}

pub fn human_bytes(b: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", units[i]) }
}

pub fn octal_unescape(s: &str) -> String {
    // /proc/mounts writes a space in a mount point as \040
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

const SKIP_FS: &[&str] = &["squashfs", "tmpfs", "devtmpfs", "overlay", "iso9660", "udf", "ramfs"];

/// One real filesystem: its mount point (cleaned), size, free space and inode use.
pub struct Mount {
    pub name: String,
    pub total: u64,
    pub avail: u64,
    pub inode_pct: f64,
}

/// Real filesystems only (no snaps, tmpfs, loop devices; a bind mount counts once). None: no /proc/mounts.
pub fn mounts(sys: &Sys) -> Option<Vec<Mount>> {
    let text = sys.read(sys.path("/proc/mounts"))?;
    let mut seen = std::collections::HashSet::new();
    let mut out = vec![];
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 || !f[0].starts_with("/dev/") || f[0].starts_with("/dev/loop") || SKIP_FS.contains(&f[2]) {
            continue;
        }
        if !seen.insert(f[0].to_string()) {
            continue; // the same filesystem mounted twice (bind mounts)
        }
        let mnt = octal_unescape(f[1]);
        let Some((total, avail, files, ffree)) = (sys.statvfs)(&mnt) else { continue };
        if total == 0 {
            continue;
        }
        let inode_pct = if files > 0 { 100.0 * (files - ffree.min(files)) as f64 / files as f64 } else { 0.0 };
        out.push(Mount { name: clean(&mnt, 80), total, avail: avail.min(total), inode_pct });
    }
    Some(out)
}

pub fn disks(sys: &Sys, lim: &Limits) -> Check {
    let Some(list) = mounts(sys) else {
        return Check::new("disks", "Disk space", Status::Unknown, "cannot read /proc/mounts");
    };
    let (mut worst, mut details, mut fullest) = (Status::Ok, vec![], (0.0f64, String::new()));
    for m in list {
        let (total, avail, inodes) = (m.total, m.avail, m.inode_pct);
        let used = 100.0 * (total - avail) as f64 / total as f64;
        let pct = used.max(inodes);
        let st = if pct >= lim.disk_bad { Status::Bad } else if pct >= lim.disk_warn { Status::Warn } else { Status::Ok };
        worst = worst.max(st);
        let name = m.name;
        let mut text = format!("{name}: {used:.0}% used, {} free of {}", human_bytes(avail), human_bytes(total));
        if inodes >= lim.disk_warn {
            text += &format!(", {inodes:.0}% of inodes used");
        }
        if pct > fullest.0 {
            fullest = (pct, name);
        }
        details.push(text);
    }
    if details.is_empty() {
        return Check::new("disks", "Disk space", Status::Unknown, "no disk filesystems found");
    }
    let summary = match worst {
        Status::Ok => format!("{} filesystem(s), fullest {} at {:.0}%", details.len(), fullest.1, fullest.0),
        _ => format!("{} is {:.0}% full", fullest.1, fullest.0),
    };
    Check::new("disks", "Disk space", worst, summary).with(details)
}

pub fn disk_errors(sys: &Sys) -> Check {
    let mut details = vec![];
    for dev in sorted_dir(&sys.path("/sys/block")) {
        let name = dev.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if name.starts_with("loop") || name.starts_with("ram") || name.starts_with("zram") {
            continue;
        }
        let Some(raw) = sys.read(dev.join("device/ioerr_cnt")) else { continue };
        let n = u64::from_str_radix(raw.trim_start_matches("0x"), 16).unwrap_or(0);
        if n > 0 {
            let model = sys.read(dev.join("device/model")).map(|m| format!(" ({})", clean(&m, 40))).unwrap_or_default();
            details.push(format!("{}{model}: {n} I/O error(s) since boot", clean(&name, 20)));
        }
    }
    if details.is_empty() {
        Check::new("disk-errors", "Disk errors", Status::Ok, "no I/O errors reported since boot")
    } else {
        Check::new("disk-errors", "Disk errors", Status::Warn,
                   format!("{} disk(s) reported I/O errors since boot: check the cable, then the disk's health (smartctl -a)", details.len()))
            .with(details)
    }
}

pub fn temperatures(sys: &Sys, lim: &Limits) -> Check {
    let (mut worst, mut details, mut hottest) = (Status::Ok, vec![], (f64::MIN, String::new()));
    for hw in sorted_dir(&sys.path("/sys/class/hwmon")) {
        let chip = sys.read(hw.join("name")).map(|n| clean(&n, 24)).unwrap_or_else(|| "?".into());
        for i in 1..=16 {
            let Some(milli) = sys.num(hw.join(format!("temp{i}_input"))) else { continue };
            let t = milli / 1000.0;
            if !(-40.0..=150.0).contains(&t) {
                continue; // a sensor that is not connected reads nonsense
            }
            let label = sys.read(hw.join(format!("temp{i}_label"))).map(|l| clean(&l, 24)).unwrap_or_else(|| format!("temp{i}"));
            let crit = sys.num(hw.join(format!("temp{i}_crit"))).map(|v| v / 1000.0).filter(|v| *v > 20.0 && *v < 150.0);
            let max = sys.num(hw.join(format!("temp{i}_max"))).map(|v| v / 1000.0).filter(|v| *v > 20.0 && *v < 150.0);
            let bad_at = crit.unwrap_or(lim.temp_bad).min(lim.temp_bad);
            let warn_at = max.unwrap_or(lim.temp_warn).min(lim.temp_warn).min(bad_at);
            let st = if t >= bad_at { Status::Bad } else if t >= warn_at { Status::Warn } else { Status::Ok };
            worst = worst.max(st);
            let name = format!("{chip} {label}");
            if t > hottest.0 {
                hottest = (t, name.clone());
            }
            let limit = crit.map(|c| format!(" (critical {c:.0} °C)")).unwrap_or_default();
            details.push(format!("{name}: {t:.0} °C{limit}"));
        }
    }
    if details.is_empty() {
        return Check::new("temperatures", "Temperatures", Status::Unknown, "no temperature sensors found");
    }
    let summary = format!("hottest: {} at {:.0} °C", hottest.1, hottest.0);
    Check::new("temperatures", "Temperatures", worst, summary).with(details)
}

/// How much of its design capacity the laptop's own battery still holds, in percent.
pub fn battery_health(sys: &Sys) -> Option<f64> {
    for ps in sorted_dir(&sys.path("/sys/class/power_supply")) {
        if sys.read(ps.join("type")).as_deref() != Some("Battery") || sys.read(ps.join("scope")).as_deref() == Some("Device") {
            continue;
        }
        let full = sys.num(ps.join("energy_full")).or_else(|| sys.num(ps.join("charge_full")))?;
        let design = sys.num(ps.join("energy_full_design")).or_else(|| sys.num(ps.join("charge_full_design")))?;
        return (design > 0.0).then(|| (100.0 * full / design).min(100.0));
    }
    None
}

pub fn battery(sys: &Sys, lim: &Limits) -> Vec<Check> {
    let mut main = None;
    let mut low_devices = vec![];
    for ps in sorted_dir(&sys.path("/sys/class/power_supply")) {
        if sys.read(ps.join("type")).as_deref() != Some("Battery") {
            continue;
        }
        let name = ps.file_name().map(|n| clean(&n.to_string_lossy(), 24)).unwrap_or_default();
        let capacity = sys.num(ps.join("capacity"));
        if sys.read(ps.join("scope")).as_deref() == Some("Device") {
            // a wireless mouse or keyboard: only its charge matters
            if let Some(c) = capacity.filter(|c| *c <= 15.0) {
                let model = sys.read(ps.join("model_name")).map(|m| clean(&m, 40)).unwrap_or(name.clone());
                low_devices.push(format!("{model}: {c:.0}% left"));
            }
            continue;
        }
        if main.is_some() {
            continue;
        }
        let full = sys.num(ps.join("energy_full")).or_else(|| sys.num(ps.join("charge_full")));
        let design = sys.num(ps.join("energy_full_design")).or_else(|| sys.num(ps.join("charge_full_design")));
        let cycles = sys.num(ps.join("cycle_count")).filter(|c| *c > 0.0);
        let state = sys.read(ps.join("status")).map(|s| clean(&s, 20)).unwrap_or_default();
        let mut details = vec![];
        if let Some(c) = capacity {
            details.push(format!("charge {c:.0}% ({})", if state.is_empty() { "?" } else { &state }));
        }
        if let Some(c) = cycles {
            details.push(format!("{c:.0} charge cycles"));
        }
        main = Some(match (full, design) {
            (Some(f), Some(d)) if d > 0.0 => {
                let health = (100.0 * f / d).min(100.0);
                let wear = 100.0 - health;
                details.insert(0, format!("holds {health:.0}% of its design capacity"));
                let st = if wear >= lim.wear_bad { Status::Bad } else if wear >= lim.wear_warn { Status::Warn } else { Status::Ok };
                let summary = match st {
                    Status::Ok => format!("{name}: {wear:.0}% worn"),
                    _ => format!("{name} is {wear:.0}% worn: it holds {health:.0}% of what it did when new; plan a replacement"),
                };
                Check::new("battery", "Battery", st, summary).with(details)
            }
            _ => Check::new("battery", "Battery", Status::Unknown, format!("{name}: design capacity not reported")).with(details),
        });
    }
    let mut out = vec![main.unwrap_or_else(|| Check::new("battery", "Battery", Status::Ok, "no battery (a desktop)"))];
    if !low_devices.is_empty() {
        out.push(Check::new("device-batteries", "Wireless devices", Status::Warn,
                            format!("{} device battery(ies) almost empty", low_devices.len())).with(low_devices));
    }
    out
}

fn meminfo(sys: &Sys) -> Option<std::collections::HashMap<String, f64>> {
    let text = sys.read(sys.path("/proc/meminfo"))?;
    Some(
        text.lines()
            .filter_map(|l| {
                let (k, v) = l.split_once(':')?;
                Some((k.to_string(), v.split_whitespace().next()?.parse::<f64>().ok()? * 1024.0))
            })
            .collect(),
    )
}

pub fn memory(sys: &Sys, lim: &Limits) -> Check {
    let Some(m) = meminfo(sys) else { return Check::new("memory", "Memory", Status::Unknown, "cannot read /proc/meminfo") };
    let (Some(total), Some(avail)) = (m.get("MemTotal"), m.get("MemAvailable")) else {
        return Check::new("memory", "Memory", Status::Unknown, "MemAvailable not reported");
    };
    let free_pct = 100.0 * avail / total;
    let swap_total = m.get("SwapTotal").copied().unwrap_or(0.0);
    let swap_used = swap_total - m.get("SwapFree").copied().unwrap_or(swap_total);
    let mut details = vec![format!("{} available of {}", human_bytes(*avail as u64), human_bytes(*total as u64))];
    if swap_total > 0.0 {
        details.push(format!("swap: {} used of {}", human_bytes(swap_used as u64), human_bytes(swap_total as u64)));
    }
    let swap_pct = if swap_total > 0.0 { 100.0 * swap_used / swap_total } else { 0.0 };
    let st = if free_pct < lim.mem_bad {
        Status::Bad
    } else if free_pct < lim.mem_warn || swap_pct > 80.0 {
        Status::Warn
    } else {
        Status::Ok
    };
    let summary = match st {
        Status::Ok => format!("{free_pct:.0}% available"),
        _ if free_pct < lim.mem_warn => format!("only {free_pct:.0}% of memory available: close something, or find what grows"),
        _ => format!("swap is {swap_pct:.0}% used: the machine has been short of memory"),
    };
    Check::new("memory", "Memory", st, summary).with(details)
}

pub fn load(sys: &Sys, lim: &Limits) -> Check {
    let Some(text) = sys.read(sys.path("/proc/loadavg")) else {
        return Check::new("load", "CPU load", Status::Unknown, "cannot read /proc/loadavg");
    };
    let v: Vec<f64> = text.split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect();
    if v.len() < 3 {
        return Check::new("load", "CPU load", Status::Unknown, "unexpected /proc/loadavg");
    }
    let cpus = sys
        .read(sys.path("/proc/cpuinfo"))
        .map(|c| c.lines().filter(|l| l.starts_with("processor")).count())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1));
    let per = v[2] / cpus as f64;
    let st = if per >= lim.load_warn { Status::Warn } else { Status::Ok };
    let summary = match st {
        Status::Ok => format!("{:.2} over 15 minutes on {cpus} CPU(s)", v[2]),
        _ => format!("{:.2} over 15 minutes on {cpus} CPU(s): something has been busy for a while", v[2]),
    };
    Check::new("load", "CPU load", st, summary).with(vec![format!("1 / 5 / 15 min: {:.2} / {:.2} / {:.2}", v[0], v[1], v[2])])
}

pub fn failed_units(sys: &Sys) -> Check {
    let mut details = vec![];
    let mut ran = false;
    for (scope, args) in [("system", vec!["--failed", "--plain", "--no-legend"]), ("user", vec!["--user", "--failed", "--plain", "--no-legend"])] {
        let Some(out) = (sys.systemctl)(&args) else { continue };
        ran = true;
        for line in out.lines() {
            // "unit load active sub description", sometimes after a "●" bullet
            let mut f = line.split_whitespace().skip_while(|w| *w == "●");
            let Some(unit) = f.next() else { continue };
            let unit = unit.trim_start_matches('●');
            let desc: Vec<&str> = f.skip(3).collect();
            let desc = clean(&desc.join(" "), 80);
            details.push(format!("{} ({scope}){}", clean(unit, 80), if desc.is_empty() { String::new() } else { format!(": {desc}") }));
        }
    }
    if !ran {
        return Check::new("services", "Services", Status::Unknown, "systemctl is not available");
    }
    if details.is_empty() {
        return Check::new("services", "Services", Status::Ok, "no failed services");
    }
    Check::new("services", "Services", Status::Bad,
               format!("{} service(s) failed: see journalctl -u <name> (add --user for user services)", details.len()))
        .with(details)
}

pub fn reboot(sys: &Sys) -> Check {
    if !sys.path("/var/run/reboot-required").exists() && !sys.path("/run/reboot-required").exists() {
        let up = sys.read(sys.path("/proc/uptime")).and_then(|u| u.split_whitespace().next()?.parse::<f64>().ok());
        let summary = up.map(|s| format!("no restart pending; up {:.1} days", s / 86400.0)).unwrap_or_else(|| "no restart pending".into());
        return Check::new("reboot", "Restart", Status::Ok, summary);
    }
    let pkgs: Vec<String> = sys
        .read(sys.path("/var/run/reboot-required.pkgs"))
        .or_else(|| sys.read(sys.path("/run/reboot-required.pkgs")))
        .map(|t| t.lines().map(|l| clean(l, 60)).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default();
    let mut unique = pkgs.clone();
    unique.sort();
    unique.dedup();
    Check::new("reboot", "Restart", Status::Warn,
               format!("a restart is pending to finish updates{}", if unique.is_empty() { String::new() } else { format!(" ({} package(s))", unique.len()) }))
        .with(unique)
}

pub fn all(sys: &Sys, lim: &Limits) -> Vec<Check> {
    let mut out = vec![disks(sys, lim), disk_errors(sys), temperatures(sys, lim)];
    out.extend(battery(sys, lim));
    out.extend([memory(sys, lim), load(sys, lim), failed_units(sys), reboot(sys)]);
    out
}

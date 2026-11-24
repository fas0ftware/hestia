use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use hestia::alerts;
use hestia::checks::{self, Limits, Status, Sys};
use hestia::cli;
use hestia::report::{self, Report};

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

fn sys(root: &Path) -> Sys {
    Sys {
        root: root.to_path_buf(),
        statvfs: Box::new(|mnt| match mnt {
            "/" => Some((100 << 30, 9 << 30, 1000, 900)),             // 91% full
            "/boot/efi" => Some((1 << 30, 1 << 29, 0, 0)),           // 50%
            "/media/my disk" => Some((1000 << 30, 900 << 30, 1000, 20)), // inodes 98% used
            _ => None,
        }),
        systemctl: Box::new(|args| {
            Some(if args.contains(&"--user") {
                "● backup.service loaded failed failed Nightly backup\n".into()
            } else {
                String::new()
            })
        }),
    }
}

fn machine() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let r = d.path();
    put(r, "proc/mounts", "/dev/nvme0n1p2 / ext4 rw 0 0\n/dev/nvme0n1p1 /boot/efi vfat rw 0 0\n\
         /dev/loop3 /snap/x/1 squashfs ro 0 0\ntmpfs /run tmpfs rw 0 0\n/dev/nvme0n1p2 /home/bind ext4 rw 0 0\n\
         /dev/sda1 /media/my\\040disk ext4 rw 0 0\n");
    put(r, "sys/block/nvme0n1/device/ioerr_cnt", "0x0\n");
    put(r, "sys/block/sda/device/ioerr_cnt", "0x3\n");
    put(r, "sys/block/sda/device/model", "JMS583\x1b[31m\n");
    put(r, "sys/block/loop0/device/ioerr_cnt", "0x9\n");
    put(r, "sys/class/hwmon/hwmon1/name", "acpitz\n");
    put(r, "sys/class/hwmon/hwmon1/temp1_input", "52000\n");
    put(r, "sys/class/hwmon/hwmon1/temp2_input", "-273000\n");
    put(r, "sys/class/hwmon/hwmon3/name", "nvme\n");
    put(r, "sys/class/hwmon/hwmon3/temp1_input", "81850\n");
    put(r, "sys/class/hwmon/hwmon3/temp1_label", "Composite\n");
    put(r, "sys/class/hwmon/hwmon3/temp1_max", "81850\n");
    put(r, "sys/class/hwmon/hwmon3/temp1_crit", "84850\n");
    put(r, "sys/class/power_supply/AC/type", "Mains\n");
    put(r, "sys/class/power_supply/BAT1/type", "Battery\n");
    put(r, "sys/class/power_supply/BAT1/charge_full", "2400000\n");
    put(r, "sys/class/power_supply/BAT1/charge_full_design", "3620000\n");
    put(r, "sys/class/power_supply/BAT1/capacity", "77\n");
    put(r, "sys/class/power_supply/BAT1/status", "Discharging\n");
    put(r, "sys/class/power_supply/BAT1/cycle_count", "0\n");
    put(r, "sys/class/power_supply/hidpp_battery_0/type", "Battery\n");
    put(r, "sys/class/power_supply/hidpp_battery_0/scope", "Device\n");
    put(r, "sys/class/power_supply/hidpp_battery_0/capacity", "5\n");
    put(r, "sys/class/power_supply/hidpp_battery_0/model_name", "MX Master 3\n");
    put(r, "proc/meminfo", "MemTotal:       16000000 kB\nMemFree:  100 kB\nMemAvailable:    1200000 kB\n\
         SwapTotal:       4000000 kB\nSwapFree:        3900000 kB\n");
    put(r, "proc/loadavg", "9.10 8.20 7.30 3/1200 4242\n");
    put(r, "proc/cpuinfo", "processor\t: 0\nmodel name\t: x\nprocessor\t: 1\nprocessor\t: 2\nprocessor\t: 3\n");
    put(r, "proc/uptime", "172800.5 100.0\n");
    d
}

#[test]
fn disks_skip_loops_tmpfs_and_bind_mounts() {
    let d = machine();
    let c = checks::disks(&sys(d.path()), &Limits::default());
    assert_eq!(c.status, Status::Bad, "{c:?}");
    assert_eq!(c.details.len(), 3);
    assert!(c.details[0].starts_with("/: 91% used"));
    assert!(c.details[2].contains("/media/my disk") && c.details[2].contains("98% of inodes"));
    assert_eq!(c.summary, "/media/my disk is 98% full");
}

#[test]
fn disk_errors_and_hostile_model_names() {
    let d = machine();
    let c = checks::disk_errors(&sys(d.path()));
    assert_eq!(c.status, Status::Warn);
    assert_eq!(c.details, vec!["sda (JMS583?[31m): 3 I/O error(s) since boot"]);
}

#[test]
fn temperatures_use_the_sensor_limits() {
    let d = machine();
    let c = checks::temperatures(&sys(d.path()), &Limits::default());
    assert_eq!(c.status, Status::Warn, "81.85 reached the nvme max, below its 84.85 critical");
    assert_eq!(c.summary, "hottest: nvme Composite at 82 °C");
    assert_eq!(c.details.len(), 2, "the -273 °C reading of an unconnected sensor is skipped");
    put(d.path(), "sys/class/hwmon/hwmon3/temp1_input", "90000\n");
    assert_eq!(checks::temperatures(&sys(d.path()), &Limits::default()).status, Status::Bad);
}

#[test]
fn battery_wear_and_device_batteries() {
    let d = machine();
    let out = checks::battery(&sys(d.path()), &Limits::default());
    assert_eq!(out[0].status, Status::Warn);
    assert!(out[0].summary.starts_with("BAT1 is 34% worn"), "{}", out[0].summary);
    assert!(!out[0].details.iter().any(|x| x.contains("cycles")), "a cycle count of 0 means not reported");
    assert_eq!(out[1].id, "device-batteries");
    assert_eq!(out[1].details, vec!["MX Master 3: 5% left"]);
    let empty = tempfile::tempdir().unwrap();
    let none = checks::battery(&sys(empty.path()), &Limits::default());
    assert_eq!((none.len(), none[0].status), (1, Status::Ok));
}

#[test]
fn memory_load_services_reboot() {
    let d = machine();
    let s = sys(d.path());
    let lim = Limits::default();
    let m = checks::memory(&s, &lim);
    assert_eq!(m.status, Status::Warn, "7.5% available");
    let l = checks::load(&s, &lim);
    assert_eq!(l.status, Status::Warn, "7.3 on 4 CPUs");
    let u = checks::failed_units(&s);
    assert_eq!(u.status, Status::Bad);
    assert_eq!(u.details, vec!["backup.service (user): Nightly backup"]);
    let r = checks::reboot(&s);
    assert_eq!((r.status, r.summary.as_str()), (Status::Ok, "no restart pending; up 2.0 days"));
    put(d.path(), "var/run/reboot-required", "*** System restart required ***\n");
    put(d.path(), "var/run/reboot-required.pkgs", "linux-base\nlibc6\nlinux-base\n");
    let r = checks::reboot(&s);
    assert_eq!(r.status, Status::Warn);
    assert_eq!(r.details, vec!["libc6", "linux-base"]);
    let no_systemctl = Sys { systemctl: Box::new(|_| None), ..sys(d.path()) };
    assert_eq!(checks::failed_units(&no_systemctl).status, Status::Unknown);
}

#[test]
fn report_counts_text_and_changes() {
    let d = machine();
    let rep = Report::new(checks::all(&sys(d.path()), &Limits::default()), 1_790_000_000);
    assert_eq!(rep.time, "2026-09-21T14:13:20Z");
    assert_eq!(rep.status, Status::Bad);
    assert_eq!(rep.counts["bad"], 2);
    let text = rep.text(false);
    assert!(text.contains("BAD      Services") && text.contains("backup.service (user)"));
    let mut before = BTreeMap::new();
    before.insert("disks".to_string(), Status::Bad);
    before.insert("memory".to_string(), Status::Bad);
    before.insert("reboot".to_string(), Status::Warn);
    let ch: Vec<(String, Status)> = report::changes(&before, &rep).into_iter().map(|(c, b)| (c.id, b)).collect();
    assert!(ch.contains(&("reboot".into(), Status::Warn)), "recovered");
    assert!(ch.contains(&("services".into(), Status::Ok)), "new failure");
    assert!(!ch.iter().any(|(id, _)| id == "disks"), "still bad: no repeat");
    assert!(!ch.iter().any(|(id, _)| id == "memory"), "bad -> warn is better, not news");
}

#[test]
fn files_are_private_and_previous_round_trips() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    let rep = Report::new(hestia::demo::checks(), 0);
    let p = d.path().join("st/last.json");
    report::save(&p, &report::previous_json(&rep)).unwrap();
    assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(fs::metadata(p.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    let back = report::load_previous(&p);
    assert_eq!(back["services"], Status::Bad);
    assert!(report::load_previous(&d.path().join("missing")).is_empty());
}

#[test]
fn alerts_title_priority_and_failure() {
    use std::sync::{Arc, Mutex};
    let rep = Report::new(hestia::demo::checks(), 0);
    let changes = report::changes(&BTreeMap::new(), &rep);
    let calls: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let c2 = calls.clone();
    let post: alerts::Post = Box::new(move |_, h, body| {
        c2.lock().unwrap().push((h.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(";"), body.to_string()));
        Ok(())
    });
    assert_eq!(alerts::send("https://ntfy.sh/t", "box", &changes, &post), changes.len());
    let calls = calls.lock().unwrap();
    assert!(calls.iter().any(|(h, b)| h.contains("Title=Hestia: Services on box") && h.contains("Priority=high") && b.contains("backup-nightly")));
    let failing: alerts::Post = Box::new(|_, _, _| Err("offline".into()));
    assert_eq!(alerts::send("https://ntfy.sh/t", "box", &changes, &failing), 0);
    assert!(alerts::valid_url("https://ntfy.sh/topic") && !alerts::valid_url("file:///etc/passwd") && !alerts::valid_url("https:///x"));
}

#[test]
fn cli_parsing() {
    let a = |v: &[&str]| cli::parse(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>(), true);
    let p = a(&["--save", "--json"]).unwrap();
    assert_eq!((p.cmd.as_str(), p.save.as_deref(), p.json), ("check", Some(""), true));
    assert_eq!(a(&["check", "--save", "/tmp/s.json", "-q"]).unwrap().save.as_deref(), Some("/tmp/s.json"));
    assert_eq!(a(&["--save", "demo"]).unwrap().cmd, "demo");
    assert!(!a(&["--no-color"]).unwrap().color);
    assert!(a(&["--ntfy"]).is_err() && a(&["--bogus"]).is_err());
    assert_eq!(checks::octal_unescape("/media/my\\040disk"), "/media/my disk");
}

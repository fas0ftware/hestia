//! A fictional machine, for screenshots and for trying Hestia anywhere.

use crate::checks::{Check, Status};

fn c(id: &str, title: &str, status: Status, summary: &str, details: &[&str]) -> Check {
    Check {
        id: id.into(),
        title: title.into(),
        status,
        summary: summary.into(),
        details: details.iter().map(|d| d.to_string()).collect(),
    }
}

pub fn checks() -> Vec<Check> {
    vec![
        c("disks", "Disk space", Status::Warn, "/ is 91% full",
          &["/: 91% used, 41.2 GiB free of 467.9 GiB", "/boot/efi: 6% used, 1.0 GiB free of 1.1 GiB", "/media/data: 54% used, 420.6 GiB free of 931.5 GiB"]),
        c("disk-errors", "Disk errors", Status::Ok, "no I/O errors reported since boot", &[]),
        c("disk-trend", "Disk trend", Status::Warn, "/ fills in about 23 days (+1.8 GiB/day over the last 14 days)",
          &["/: +1.8 GiB/day, full in about 23 days", "/boot/efi: steady", "/media/data: +240.0 MiB/day, full in about 1795 days"]),
        c("temperatures", "Temperatures", Status::Ok, "hottest: k10temp Tctl at 61 °C",
          &["acpitz temp1: 52 °C (critical 105 °C)", "nvme Composite: 43 °C (critical 85 °C)", "k10temp Tctl: 61 °C", "amdgpu edge: 49 °C"]),
        c("battery", "Battery", Status::Warn, "BAT0 is 34% worn: it holds 66% of what it did when new; plan a replacement",
          &["holds 66% of its design capacity", "charge 80% (Discharging)", "612 charge cycles",
            "61 days ago it held 71%, now 66% (-5.0 points)"]),
        c("device-batteries", "Wireless devices", Status::Warn, "1 device battery(ies) almost empty", &["MX Master 3: 8% left"]),
        c("memory", "Memory", Status::Ok, "48% available", &["7.4 GiB available of 15.3 GiB", "swap: 512.0 MiB used of 4.0 GiB"]),
        c("load", "CPU load", Status::Ok, "0.84 over 15 minutes on 12 CPU(s)", &["1 / 5 / 15 min: 1.10 / 0.92 / 0.84"]),
        c("services", "Services", Status::Bad, "1 service(s) failed: see journalctl -u <name> (add --user for user services)",
          &["backup-nightly.service (user): Nightly backup"]),
        c("reboot", "Restart", Status::Warn, "a restart is pending to finish updates (2 package(s))", &["linux-image-6.8.0-49-generic", "libc6"]),
    ]
}

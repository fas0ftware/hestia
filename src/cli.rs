//! Command line. No argument parser crate: the surface is small.

use std::io::IsTerminal;
use std::path::PathBuf;

use crate::checks::{self, Limits, Sys};
use crate::report::{self, Report};
use crate::{alerts, demo, VERSION};

const HELP: &str = "hestia: is this laptop healthy? Disks, temperatures, battery, memory, load, failed services.

usage:
  hestia [check] [--json] [--save [PATH]] [--ntfy URL] [-q] [--no-color]
  hestia demo [--json]
  hestia --version

  --save [PATH]  write the status file Olympos reads (default ~/.local/state/hestia/status.json)
  --ntfy URL     push checks that got worse or recovered since the last run (or HESTIA_NTFY_URL)
  -q, --quiet    print nothing (for timers)

thresholds (environment): HESTIA_DISK_WARN=85 HESTIA_DISK_BAD=95 HESTIA_TEMP_WARN=85 HESTIA_TEMP_BAD=95
  HESTIA_WEAR_WARN=30 HESTIA_WEAR_BAD=50 HESTIA_MEM_WARN=10 HESTIA_MEM_BAD=5 HESTIA_LOAD_WARN=1.5";

#[derive(Debug, Default, PartialEq)]
pub struct Args {
    pub cmd: String,
    pub json: bool,
    pub save: Option<String>,
    pub ntfy: String,
    pub quiet: bool,
    pub color: bool,
    pub root: String,
}

pub fn parse(argv: &[String], tty: bool) -> Result<Args, String> {
    let mut a = Args { cmd: "check".into(), color: tty, root: "/".into(), ..Default::default() };
    let mut it = argv.iter().peekable();
    while let Some(x) = it.next() {
        match x.as_str() {
            "check" | "demo" => a.cmd = x.clone(),
            "--json" => a.json = true,
            "-q" | "--quiet" => a.quiet = true,
            "--no-color" => a.color = false,
            "--save" => {
                a.save = Some(match it.peek() {
                    Some(p) if !p.starts_with('-') && *p != "check" && *p != "demo" => it.next().cloned().unwrap_or_default(),
                    _ => String::new(),
                })
            }
            "--ntfy" => a.ntfy = it.next().cloned().ok_or("--ntfy needs a URL")?,
            "--root" => a.root = it.next().cloned().ok_or("--root needs a directory")?,
            "-h" | "--help" => a.cmd = "help".into(),
            "-V" | "--version" => a.cmd = "version".into(),
            other => return Err(format!("unknown argument {other:?} (see hestia --help)")),
        }
    }
    Ok(a)
}

fn env_num(name: &str, default: f64) -> Result<f64, String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v.trim().parse::<f64>().ok().filter(|n| n.is_finite() && *n >= 0.0)
            .ok_or(format!("{name} must be a number")),
        _ => Ok(default),
    }
}

pub fn limits_from_env() -> Result<Limits, String> {
    let d = Limits::default();
    Ok(Limits {
        disk_warn: env_num("HESTIA_DISK_WARN", d.disk_warn)?,
        disk_bad: env_num("HESTIA_DISK_BAD", d.disk_bad)?,
        temp_warn: env_num("HESTIA_TEMP_WARN", d.temp_warn)?,
        temp_bad: env_num("HESTIA_TEMP_BAD", d.temp_bad)?,
        wear_warn: env_num("HESTIA_WEAR_WARN", d.wear_warn)?,
        wear_bad: env_num("HESTIA_WEAR_BAD", d.wear_bad)?,
        mem_warn: env_num("HESTIA_MEM_WARN", d.mem_warn)?,
        mem_bad: env_num("HESTIA_MEM_BAD", d.mem_bad)?,
        load_warn: env_num("HESTIA_LOAD_WARN", d.load_warn)?,
    })
}

pub fn state_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("hestia")
}

fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname").map(|h| checks::clean(h.trim(), 40)).unwrap_or_default()
}

pub fn run(argv: &[String]) -> i32 {
    let args = match parse(argv, std::io::stdout().is_terminal()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("hestia: {e}");
            return 2;
        }
    };
    match args.cmd.as_str() {
        "help" => {
            println!("{HELP}");
            return 0;
        }
        "version" => {
            println!("hestia {VERSION}");
            return 0;
        }
        _ => {}
    }
    let checks = if args.cmd == "demo" {
        demo::checks()
    } else {
        let lim = match limits_from_env() {
            Ok(l) => l,
            Err(e) => {
                eprintln!("hestia: {e}");
                return 2;
            }
        };
        let mut sys = Sys::real();
        sys.root = PathBuf::from(&args.root);
        checks::all(&sys, &lim)
    };
    let rep = Report::new(checks, report::now_secs());
    if !args.quiet {
        if args.json {
            println!("{}", serde_json::to_string_pretty(&rep.json()).unwrap_or_default());
        } else {
            print!("{}", rep.text(args.color));
        }
    }
    if args.cmd == "demo" {
        return 0;
    }
    if let Some(p) = &args.save {
        let path = if p.is_empty() { state_dir().join("status.json") } else { PathBuf::from(p) };
        if let Err(e) = report::save(&path, &rep.json()) {
            eprintln!("hestia: cannot write {}: {e}", path.display());
            return 1;
        }
    }
    let url = if args.ntfy.is_empty() { std::env::var("HESTIA_NTFY_URL").unwrap_or_default().trim().to_string() } else { args.ntfy.clone() };
    if !url.is_empty() {
        if !alerts::valid_url(&url) {
            eprintln!("hestia: the ntfy address must be an http(s) URL");
            return 2;
        }
        let last = state_dir().join("last.json");
        let changes = report::changes(&report::load_previous(&last), &rep);
        let sent = alerts::send(&url, &hostname(), &changes, &alerts::ureq_post());
        // Remember the new state only when every change was delivered, so a failed push is retried.
        if sent == changes.len() {
            if let Err(e) = report::save(&last, &report::previous_json(&rep)) {
                eprintln!("hestia: cannot write {}: {e}", last.display());
            }
        }
    }
    0
}

# Hestia

![Version](https://img.shields.io/badge/version-0.1.0-blue)
![Rust](https://img.shields.io/badge/rust-1.75%2B-orange)
![Root](https://img.shields.io/badge/root-not%20needed-brightgreen)
![License](https://img.shields.io/badge/license-MIT-green)

**Hestia answers one question: is this laptop healthy?** Disk space, disks
that report I/O errors, temperatures, how worn the battery is, a wireless
mouse about to die, memory and swap, a CPU that has been busy for too long,
services that failed, and a restart that updates are waiting for.

It reads `/proc`, `/sys` and `systemctl`, needs no root and changes nothing.
One static-ish binary, a status file for the
[Olympos](https://github.com/fas0ftware/olympos) dashboard, and phone alerts
(ntfy) **only when something changes**: a check that gets worse, or recovers.

```
$ hestia demo
Hestia 0.1.0 · 2026-11-24T16:41:43Z

WARN     Disk space         / is 91% full
                            /: 91% used, 41.2 GiB free of 467.9 GiB
                            /boot/efi: 6% used, 1.0 GiB free of 1.1 GiB
                            /media/data: 54% used, 420.6 GiB free of 931.5 GiB
OK       Disk errors        no I/O errors reported since boot
OK       Temperatures       hottest: k10temp Tctl at 61 °C
WARN     Battery            BAT0 is 34% worn: it holds 66% of what it did when new; plan a replacement
                            holds 66% of its design capacity
                            charge 80% (Discharging)
                            612 charge cycles
WARN     Wireless devices   1 device battery(ies) almost empty
                            MX Master 3: 8% left
OK       Memory             48% available
OK       CPU load           0.84 over 15 minutes on 12 CPU(s)
BAD      Services           1 service(s) failed: see journalctl -u <name> (add --user for user services)
                            backup-nightly.service (user): Nightly backup
WARN     Restart            a restart is pending to finish updates (2 package(s))
                            linux-image-6.8.0-49-generic
                            libc6

4 ok · 4 warn · 1 bad · 0 unknown
```
<sub>(fictional machine)</sub>

## Checks

| Check | Warn | Bad | Read from |
|---|---|---|---|
| Disk space | 85% of space *or* inodes | 95% | `/proc/mounts` + `statvfs`; real filesystems only (no snaps, tmpfs, loop devices; a bind mount counts once) |
| Disk errors | any I/O error since boot | | `/sys/block/*/device/ioerr_cnt` |
| Temperatures | the sensor's own `max`, at most 85 °C | its `crit`, at most 95 °C | `/sys/class/hwmon`; unconnected sensors (−273 °C) are skipped |
| Battery | 30% worn | 50% worn | `/sys/class/power_supply`: full vs design capacity, charge, cycles |
| Wireless devices | a mouse or keyboard battery at 15% or less | | power supplies with `scope=Device` |
| Memory | under 10% available, or swap over 80% used | under 5% | `/proc/meminfo` (`MemAvailable`, not "free") |
| CPU load | 15-minute load over 1.5 per CPU | | `/proc/loadavg`, `/proc/cpuinfo` |
| Services | | any failed system or user unit | `systemctl --failed` and `systemctl --user --failed` |
| Restart | updates wait for a restart | | `/run/reboot-required(.pkgs)` (Debian, Ubuntu) |

A check that cannot be read (no sensors in a VM, no systemd) is `unknown` and
does not make the overall status worse. Every limit can be changed in the
environment: `HESTIA_DISK_WARN`, `HESTIA_DISK_BAD`, `HESTIA_TEMP_WARN`,
`HESTIA_TEMP_BAD`, `HESTIA_WEAR_WARN`, `HESTIA_WEAR_BAD`, `HESTIA_MEM_WARN`,
`HESTIA_MEM_BAD`, `HESTIA_LOAD_WARN`.

## Install

```sh
git clone https://github.com/fas0ftware/hestia && cd hestia
cargo build --release
./target/release/hestia
./contrib/install.sh target/release/hestia   # ~/.local/bin/hestia + a user timer every 15 minutes
```

`./contrib/install.sh --remove` takes it away again. Without a local Rust
toolchain, build in a container:
`docker run --rm -v "$PWD":/src -w /src rust:1-slim cargo build --release`.

### Phone alerts

```sh
# ~/.config/hestia/env
HESTIA_NTFY_URL=https://ntfy.sh/a-long-random-topic
```

The timer then pushes a message when a check turns `warn` or `bad` ("Services
on laptop: mnemosyne-maintain.service failed"), and again when it is `ok`
("recovered"). A check that stays bad is not repeated every 15 minutes. If a
push fails, the change is sent again on the next run.

## Commands

| Command | Does |
|---|---|
| `hestia` / `hestia check` | Run every check; `--json`, `--save [PATH]` (status file), `--ntfy URL`, `-q`, `--no-color` |
| `hestia demo [--json]` | A fictional machine |
| `hestia --version` / `--help` | |

The status file (`~/.local/state/hestia/status.json`, mode `0600`) is the
`--json` report: version, time, overall `status`, `counts`, and every check
with `id`, `title`, `status`, `summary` and `details`.

## Security

- **Read-only, no root.** Files under `/proc` and `/sys`, `statvfs`, and
  `systemctl --failed`; nothing is written except Hestia's own state.
- **Device text is untrusted.** Sensor labels, disk models and service
  descriptions are reduced to printable ASCII and capped before they are
  printed, saved or pushed (a USB disk can name itself anything, including
  terminal escape sequences).
- **Owner-only files** (`0600` in a `0700` directory), written atomically.

## Tests

```sh
cargo test        # 9 tests on a fake /sys and /proc, no root
cargo clippy --all-targets -- -D warnings
```

The tests build a fake machine in a temporary directory: mounts with a space
in the name, a snap and a bind mount, a disk with I/O errors and an escape
sequence in its model, an NVMe at its `max` temperature, a disconnected sensor,
a worn battery, a nearly empty mouse, low memory, a busy CPU, a failed user
service, a pending restart; plus the change detection behind the alerts, file
permissions and argument parsing. CI also runs a real check of the runner and
`cargo audit`.

On the author's laptop the first real run found a failed user service (a
backup maintenance job) and an external SSD that had reported three I/O
errors since boot.

## Roadmap

- SMART health of disks (needs root or a `smartctl` drop-in: optional).
- Battery health history, to see wear over months.
- Fan speeds and CPU throttling.

## License

[MIT](LICENSE)

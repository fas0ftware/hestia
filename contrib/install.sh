#!/bin/sh
# Install Hestia for this user: the binary in ~/.local/bin and a systemd user timer (no root).
# ./contrib/install.sh [path/to/hestia]   install or update (builds with cargo if no binary is given)
# ./contrib/install.sh --remove           stop and remove the timer and the binary (keeps the status file)
set -eu
units="$HOME/.config/systemd/user"
bin="$HOME/.local/bin"
if [ "${1:-}" = "--remove" ]; then
    systemctl --user disable --now hestia.timer 2>/dev/null || true
    rm -f "$units/hestia.service" "$units/hestia.timer" "$bin/hestia"
    systemctl --user daemon-reload
    echo "Hestia removed (status kept in ~/.local/state/hestia)."
    exit 0
fi
src="${1:-}"
if [ -z "$src" ]; then
    cargo build --release
    src=target/release/hestia
fi
mkdir -p "$units" "$bin"
install -m 0755 "$src" "$bin/hestia"
cp contrib/hestia.service contrib/hestia.timer "$units/"
systemctl --user daemon-reload
systemctl --user enable --now hestia.timer
systemctl --user start hestia.service
echo "Hestia runs every 15 minutes; status: ~/.local/state/hestia/status.json"
"$bin/hestia"

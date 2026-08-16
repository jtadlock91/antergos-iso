#!/bin/bash
# ============================================================
# distro-tool-firstboot.sh
# ============================================================
# Runs once, on the very first login after install: builds distro-tool
# from source and runs `distro-tool setup`, then removes itself so it
# never runs again.
#
# Why this needs a visible terminal and one password prompt, rather than
# being fully silent: distro-tool needs sudo internally for a lot of its
# work, and `makepkg` (used to build ananicy-cpp/irqbalance cleanly,
# without systemd) explicitly refuses to ever run as root. That mix of
# privilege levels can't be collapsed into a single silent background
# process running as one user — it genuinely needs a real person present
# once to authenticate.
#
# Everything after that first password entry is fully automatic.
# ============================================================

set -uo pipefail

MARKER="$HOME/.local/share/distro-tool/firstboot-done"
LOG="$HOME/.local/share/distro-tool/firstboot.log"
SRC_DIR="/usr/local/src/distro-tool"
AUTOSTART_FILE="$HOME/.config/autostart/distro-tool-firstboot.desktop"

mkdir -p "$(dirname "$MARKER")"

# Already completed successfully — shouldn't normally reach here since
# the autostart entry gets removed on success, but this is a safe
# defense-in-depth check regardless.
if [ -f "$MARKER" ]; then
    exit 0
fi

echo "=========================================="
echo " Setting up your system (one-time)"
echo "=========================================="
echo ""
echo "This builds and runs a few automatic checks — performance tuning,"
echo "snapshot protection if you're on Btrfs, and a daily cleanup tool."
echo "It happens once. After this, it's done."
echo ""
echo "You'll need to enter your password once to continue."
echo ""

{
    echo ""
    echo "=== distro-tool first-boot setup: $(date) ==="
} >> "$LOG"

# Authenticate once, then keep the sudo timestamp alive in the
# background for the duration of the run — this is what lets every
# individual `sudo` call inside distro-tool proceed without prompting
# again and again as it works through each check.
if ! sudo -v; then
    echo ""
    echo "Couldn't authenticate — nothing was changed. This will run again"
    echo "at your next login."
    echo "Setup aborted: sudo authentication failed." >> "$LOG"
    sleep 5
    exit 1
fi

( while true; do sudo -n true; sleep 60; done ) &
KEEPALIVE_PID=$!
trap 'kill "$KEEPALIVE_PID" 2>/dev/null' EXIT

# Build distro-tool if it isn't already installed. This step must run
# as the real user, never as root — makepkg (invoked internally by some
# checks) refuses to run as root by design.
if ! command -v distro-tool &>/dev/null; then
    echo "Building distro-tool (this can take a minute)..."
    echo "--- build output ---" >> "$LOG"

    if [ ! -d "$SRC_DIR" ]; then
        echo "Source directory $SRC_DIR not found — cannot build."
        echo "ERROR: $SRC_DIR missing" >> "$LOG"
        sleep 5
        exit 1
    fi

    if (cd "$SRC_DIR" && cargo build --release) >> "$LOG" 2>&1; then
        sudo install -Dm755 "$SRC_DIR/target/release/distro-tool" /usr/local/bin/distro-tool
        echo "Build succeeded."
        echo "Build succeeded." >> "$LOG"
    else
        echo ""
        echo "Build failed — see $LOG for details."
        echo "This will try again at your next login."
        echo "Build FAILED." >> "$LOG"
        sleep 8
        exit 1
    fi
fi

echo ""
echo "Running setup checks..."
echo "--- distro-tool setup output ---" >> "$LOG"

if distro-tool setup 2>&1 | tee -a "$LOG"; then
    echo ""
    echo "All done — your system is ready."
    echo "Setup completed successfully." >> "$LOG"
    touch "$MARKER"
    rm -f "$AUTOSTART_FILE"
    command -v notify-send &>/dev/null && notify-send "System setup complete" \
        "Performance tuning, snapshots, and daily cleanup are all configured." 2>/dev/null
    echo ""
    echo "This window will close in a few seconds."
    sleep 5
else
    echo ""
    echo "Some checks reported problems — see $LOG for details."
    echo "This will run again at your next login to retry."
    echo "Setup reported errors." >> "$LOG"
    sleep 8
fi

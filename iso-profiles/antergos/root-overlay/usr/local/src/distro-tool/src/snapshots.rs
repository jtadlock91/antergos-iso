use crate::checks::CheckStatus;
use crate::util::*;
use anyhow::Result;

// -------------------------------------------------------
// Bootable snapshot protection: Timeshift + grub-btrfsd.
//
// Deliberately NOT snapper, despite that being the standard choice on
// this person's other (systemd) machines: snapshots created with snapper
// specifically fail to boot on dinit systems (confirmed via an Artix
// forum report and the grub-btrfs maintainers themselves — no dinit
// support exists upstream). Timeshift is the dinit-compatible way to
// achieve the same actual goal: bootable rollback points.
//
// No official dinit service exists for grub-btrfsd either. The one
// below is translated directly from grub-btrfs's own upstream systemd
// service (ExecStart=/usr/bin/grub-btrfsd --syslog /.snapshots), swapping
// the default Snapper-oriented path argument for the --timeshift-auto
// flag the daemon documents for exactly this situation.
//
// This check is entirely conditional on root actually being Btrfs — it
// silently reports OK-and-skipped on ext4/anything else, never tries to
// convert a filesystem or force this onto a system that isn't set up
// for it.
// -------------------------------------------------------

fn root_fstype() -> Result<String> {
    let out = run("findmnt", &["-no", "FSTYPE", "/"])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn root_uuid() -> Result<String> {
    let out = run("findmnt", &["-no", "UUID", "/"])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn check_snapshots(apply: bool) -> Result<CheckStatus> {
    let fstype = root_fstype().unwrap_or_default();
    if fstype != "btrfs" {
        return Ok(CheckStatus::Ok(format!(
            "root filesystem is {fstype} (not Btrfs) — bootable snapshots not applicable, skipped"
        )));
    }

    let timeshift_ok = package_installed("timeshift");
    let grub_btrfs_ok = package_installed("grub-btrfs");
    let service_running = dinit_boot_enabled("grub-btrfsd");

    if timeshift_ok && grub_btrfs_ok && service_running {
        return Ok(CheckStatus::Ok(
            "Btrfs root detected — Timeshift + grub-btrfsd are installed and running".to_string(),
        ));
    }

    let problem = "Btrfs root detected, but bootable snapshot protection (Timeshift + \
                    grub-btrfsd) isn't fully set up"
        .to_string();

    if !apply {
        return Ok(CheckStatus::NeedsAttention(problem));
    }

    if !timeshift_ok {
        let install = run("sudo", &["pacman", "-S", "--needed", "--noconfirm", "timeshift"])?;
        if !install.status.success() {
            return Ok(CheckStatus::NeedsAttention(format!(
                "{problem} — timeshift install failed: {}",
                String::from_utf8_lossy(&install.stderr).trim()
            )));
        }
    }
    if !grub_btrfs_ok {
        let install = run(
            "sudo",
            &["pacman", "-S", "--needed", "--noconfirm", "grub-btrfs", "inotify-tools"],
        )?;
        if !install.status.success() {
            return Ok(CheckStatus::NeedsAttention(format!(
                "{problem} — grub-btrfs install failed: {}",
                String::from_utf8_lossy(&install.stderr).trim()
            )));
        }
    }

    // Configure Timeshift for BTRFS mode, non-interactively. Retention
    // matches the spirit of this person's other machines' snapper setup
    // (3 hourly / 5 daily, no weekly/monthly) translated into Timeshift's
    // own schema.
    let uuid = root_uuid().unwrap_or_default();
    let timeshift_config = format!(
        r#"{{
  "backup_device_uuid" : "{uuid}",
  "parent_device_uuid" : "",
  "do_first_run" : "false",
  "btrfs_mode" : "true",
  "include_btrfs_home_for_backup" : "false",
  "include_btrfs_home_for_restore" : "false",
  "stop_cron_emails" : "true",
  "schedule_monthly" : "false",
  "schedule_weekly" : "false",
  "schedule_daily" : "true",
  "schedule_hourly" : "true",
  "schedule_boot" : "false",
  "count_monthly" : "0",
  "count_weekly" : "0",
  "count_daily" : "5",
  "count_hourly" : "3",
  "count_boot" : "0",
  "exclude" : [],
  "exclude-apps" : []
}}
"#
    );
    write_privileged_file("/etc/timeshift/timeshift.json", &timeshift_config)?;

    // grub-btrfsd dinit service — hand-written, no upstream dinit support
    // exists. --timeshift-auto tells it to watch Timeshift's snapshot
    // directory instead of Snapper's default /.snapshots.
    let service =
        "type = process\ncommand = /usr/bin/grub-btrfsd --syslog --timeshift-auto\nrestart = true\n";
    write_privileged_file("/etc/dinit.d/grub-btrfsd", service)?;
    dinit_enable("grub-btrfsd")?;
    dinit_start("grub-btrfsd")?;

    // Create one snapshot now so there's an immediate bootable rollback
    // point and grub-btrfsd has something to pick up right away, rather
    // than waiting for the first scheduled run.
    let _ = run(
        "sudo",
        &[
            "timeshift",
            "--create",
            "--btrfs",
            "--comments",
            "distro-tool initial snapshot",
            "--scripted",
        ],
    );
    let _ = run("sudo", &["grub-mkconfig", "-o", "/boot/grub/grub.cfg"]);

    Ok(CheckStatus::Fixed(format!(
        "{problem} — installed, configured, and created an initial snapshot"
    )))
}

pub fn all_snapshot_checks() -> Vec<(&'static str, fn(bool) -> Result<CheckStatus>)> {
    vec![("Bootable Btrfs snapshots", check_snapshots)]
}

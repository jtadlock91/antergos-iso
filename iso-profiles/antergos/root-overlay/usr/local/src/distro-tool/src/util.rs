use anyhow::{Context, Result};
use std::fs;
use std::process::Command;

pub fn run(cmd: &str, args: &[&str]) -> Result<std::process::Output> {
    Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("failed to run `{cmd} {}`", args.join(" ")))
}

/// Run a command with its working directory set — used for makepkg builds,
/// which must happen in a specific directory containing the PKGBUILD.
pub fn run_in(dir: &std::path::Path, cmd: &str, args: &[&str]) -> Result<std::process::Output> {
    Command::new(cmd)
        .args(args)
        .current_dir(dir)
        .output()
        .with_context(|| format!("failed to run `{cmd} {}` in {dir:?}", args.join(" ")))
}

pub fn command_succeeds(cmd: &str, args: &[&str]) -> bool {
    run(cmd, args).map(|o| o.status.success()).unwrap_or(false)
}

pub fn package_installed(name: &str) -> bool {
    command_succeeds("pacman", &["-Qi", name])
}

/// Get the login name of the person who actually needs their system fixed,
/// even when this tool is invoked through sudo.
pub fn real_username() -> Result<String> {
    if let Ok(u) = std::env::var("SUDO_USER") {
        if !u.is_empty() {
            return Ok(u);
        }
    }
    let out = run("whoami", &[])?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn real_home() -> Result<std::path::PathBuf> {
    let user = real_username()?;
    let getent = run("getent", &["passwd", &user])?;
    let line = String::from_utf8_lossy(&getent.stdout);
    let home = line
        .trim()
        .split(':')
        .nth(5)
        .unwrap_or("")
        .to_string();
    if home.is_empty() {
        anyhow::bail!("could not determine home directory for {user}");
    }
    Ok(std::path::PathBuf::from(home))
}

/// Write content to a root-owned file via a temp file + `sudo mv`, since
/// most of the paths this tool edits (/etc/pacman.conf, /etc/dinit.d/*)
/// aren't user-writable.
pub fn write_privileged_file(path: &str, content: &str) -> Result<()> {
    let tmp = format!("/tmp/distro-tool-{}", path.replace('/', "_"));
    fs::write(&tmp, content).with_context(|| format!("failed to write temp file for {path}"))?;
    let mv = run("sudo", &["mv", &tmp, path])?;
    if !mv.status.success() {
        anyhow::bail!(
            "failed to install {path}: {}",
            String::from_utf8_lossy(&mv.stderr).trim()
        );
    }
    Ok(())
}

/// Check whether a binary at the given path is linked against libsystemd.
/// This is the exact check that caught ananicy-cpp and irqbalance pulling
/// in CachyOS's systemd-linked builds instead of dinit-safe ones.
pub fn linked_against_systemd(binary_path: &str) -> bool {
    match run("ldd", &[binary_path]) {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .to_lowercase()
            .contains("libsystemd"),
        Err(_) => false,
    }
}

/// Is a dinit service currently symlinked into boot.d (i.e. will start
/// automatically at boot)?
pub fn dinit_boot_enabled(service: &str) -> bool {
    std::path::Path::new(&format!("/etc/dinit.d/boot.d/{service}")).exists()
}

pub fn dinit_enable(service: &str) -> Result<()> {
    let link = run(
        "sudo",
        &[
            "ln",
            "-sf",
            &format!("/etc/dinit.d/{service}"),
            &format!("/etc/dinit.d/boot.d/{service}"),
        ],
    )?;
    if !link.status.success() {
        anyhow::bail!(
            "failed to enable {service}: {}",
            String::from_utf8_lossy(&link.stderr).trim()
        );
    }
    Ok(())
}

pub fn dinit_disable(service: &str) -> Result<()> {
    if dinit_boot_enabled(service) {
        run("sudo", &["rm", &format!("/etc/dinit.d/boot.d/{service}")])?;
    }
    Ok(())
}

pub fn dinit_start(service: &str) -> Result<()> {
    run("sudo", &["dinitctl", "start", service])?;
    Ok(())
}

/// Does the current user's own crontab already contain a line matching
/// this substring? Uses the real user's crontab (no sudo) — this is
/// deliberate: system-clean operates on the user's own home directory
/// and must run as them, not as root, or it'd create root-owned files
/// in their Downloads/Documents.
pub fn user_crontab_has(needle: &str) -> bool {
    match run("crontab", &["-l"]) {
        Ok(out) => String::from_utf8_lossy(&out.stdout).contains(needle),
        Err(_) => false,
    }
}

pub fn add_user_cron_entry(line: &str) -> Result<()> {
    let existing = run("crontab", &["-l"])
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let mut new_crontab = existing;
    if !new_crontab.is_empty() && !new_crontab.ends_with('\n') {
        new_crontab.push('\n');
    }
    new_crontab.push_str(line);
    new_crontab.push('\n');

    let tmp = "/tmp/distro-tool-crontab";
    std::fs::write(tmp, &new_crontab)?;
    let result = run("sh", &["-c", &format!("crontab {tmp}")])?;
    let _ = std::fs::remove_file(tmp);
    if !result.status.success() {
        anyhow::bail!(
            "failed to update crontab: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        );
    }
    Ok(())
}

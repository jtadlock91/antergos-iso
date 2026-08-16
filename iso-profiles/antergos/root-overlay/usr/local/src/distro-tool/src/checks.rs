use crate::util::{command_succeeds, real_username, run};
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Outcome of a single check. Every check reports one of these regardless
/// of whether it was allowed to fix anything — this is what lets `setup`
/// and `doctor` share the exact same logic.
pub enum CheckStatus {
    /// Nothing wrong, system already in a good state.
    Ok(String),
    /// Was wrong, and this run fixed it.
    Fixed(String),
    /// Wrong, but not fixed (either apply=false, or fix itself failed).
    NeedsAttention(String),
}

impl CheckStatus {
    pub fn label(&self) -> &'static str {
        match self {
            CheckStatus::Ok(_) => "OK",
            CheckStatus::Fixed(_) => "FIXED",
            CheckStatus::NeedsAttention(_) => "ATTENTION",
        }
    }

    pub fn message(&self) -> &str {
        match self {
            CheckStatus::Ok(m) | CheckStatus::Fixed(m) | CheckStatus::NeedsAttention(m) => m,
        }
    }
}

// -------------------------------------------------------
// Check 1: login shell exists and is registered in /etc/shells
//
// This is the exact bug that locks people out of a fresh Antergos NeXT
// install: Calamares sets zsh (or another shell) as the default without
// installing the package for it, and pam_shells.so rejects the login
// before the password is even checked.
//
// The fix here PREFERS actually providing whatever shell is configured
// (installing the missing package) over silently downgrading to bash —
// someone who chose zsh shouldn't have that choice quietly overridden
// just because the package happened to be missing. Bash is only a last
// resort if the configured shell genuinely can't be installed.
// -------------------------------------------------------
fn shell_registered(path: &str) -> bool {
    fs::read_to_string("/etc/shells")
        .map(|c| c.lines().any(|l| l.trim() == path))
        .unwrap_or(false)
}

fn register_shell(path: &str) -> Result<()> {
    let existing = fs::read_to_string("/etc/shells").unwrap_or_default();
    if existing.lines().any(|l| l.trim() == path) {
        return Ok(());
    }
    let mut new_contents = existing;
    if !new_contents.ends_with('\n') && !new_contents.is_empty() {
        new_contents.push('\n');
    }
    new_contents.push_str(path);
    new_contents.push('\n');
    crate::util::write_privileged_file("/etc/shells", &new_contents)
}

pub fn check_shell(apply: bool) -> Result<CheckStatus> {
    let username = real_username()?;

    let getent = run("getent", &["passwd", &username])?;
    if !getent.status.success() {
        return Ok(CheckStatus::NeedsAttention(format!(
            "could not look up user '{username}' in /etc/passwd"
        )));
    }
    let line = String::from_utf8_lossy(&getent.stdout);
    let shell_path = line.trim().split(':').nth(6).unwrap_or("").to_string();

    let exists = Path::new(&shell_path).exists();
    let registered = shell_registered(&shell_path);

    if exists && registered {
        return Ok(CheckStatus::Ok(format!(
            "login shell '{shell_path}' is installed and registered"
        )));
    }

    let problem = format!(
        "login shell '{shell_path}' is {} — logging in would fail",
        if !exists { "not installed" } else { "not registered in /etc/shells" }
    );

    if !apply {
        return Ok(CheckStatus::NeedsAttention(problem));
    }

    // If the binary's just missing, try installing the package that
    // matches its filename first (zsh -> zsh, fish -> fish, etc.) —
    // this covers the overwhelming majority of real shells.
    if !exists {
        let shell_name = Path::new(&shell_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if !shell_name.is_empty() {
            let _ = run("sudo", &["pacman", "-S", "--needed", "--noconfirm", &shell_name]);
        }
    }

    if Path::new(&shell_path).exists() {
        // Binary's there now (either it always was, or we just installed
        // it) — make sure /etc/shells actually lists it rather than
        // trusting the package's install script to have done so.
        register_shell(&shell_path)?;
        return Ok(CheckStatus::Fixed(format!(
            "{problem} — installed/registered '{shell_path}', shell now works"
        )));
    }

    // Genuinely couldn't provide the configured shell — fall back to
    // bash so login can never be permanently broken. This only happens
    // if the shell isn't a real installable package at all.
    let fix = run("sudo", &["chsh", "-s", "/bin/bash", &username])?;
    if fix.status.success() {
        Ok(CheckStatus::Fixed(format!(
            "{problem} — couldn't install it, fell back to /bin/bash so login works"
        )))
    } else {
        Ok(CheckStatus::NeedsAttention(format!(
            "{problem} — attempted fix failed: {}",
            String::from_utf8_lossy(&fix.stderr).trim()
        )))
    }
}

// -------------------------------------------------------
// Check 2: Artix's own repos (system/world/galaxy/antergos-pkgs/lib32)
// are listed above any CachyOS repo blocks in pacman.conf.
//
// Wrong order here is what caused the entire cascade this tool exists
// to prevent: pacman silently preferring CachyOS's systemd-linked
// builds over Artix's own dinit-safe ones for any package that exists
// in both places.
// -------------------------------------------------------
const ANTERGOS_OVERRIDE: &[&str] = &["antergos-pkgs"];
const ARTIX_HIGH_PRIORITY: &[&str] = &["system", "world", "galaxy", "lib32"];
const ARCH_MID_PRIORITY: &[&str] = &["multilib"];

fn section_priority(name: &str) -> u8 {
    if ANTERGOS_OVERRIDE.contains(&name) {
        // This distro's own override repo (branding, Calamares config,
        // and — critically — patched packages like the dinit-aware
        // pipewire launcher) must win over even Artix's own repos, or
        // its deliberate patches get silently shadowed by generic
        // upstream versions of the same package names.
        0
    } else if ARTIX_HIGH_PRIORITY.contains(&name) {
        1
    } else if ARCH_MID_PRIORITY.contains(&name) {
        2
    } else if name.starts_with("cachyos") {
        4
    } else {
        3
    }
}

struct ConfSection {
    header: String, // e.g. "[system]" — empty string for the pre-amble before [options]
    body: Vec<String>,
}

fn parse_pacman_conf(contents: &str) -> Vec<ConfSection> {
    let mut sections: Vec<ConfSection> = Vec::new();
    let mut current = ConfSection {
        header: String::new(),
        body: Vec::new(),
    };

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            sections.push(current);
            current = ConfSection {
                header: line.to_string(),
                body: Vec::new(),
            };
        } else {
            current.body.push(line.to_string());
        }
    }
    sections.push(current);
    sections
}

fn section_name(header: &str) -> String {
    header.trim().trim_start_matches('[').trim_end_matches(']').to_string()
}

pub fn check_repo_order(apply: bool) -> Result<CheckStatus> {
    let path = "/etc/pacman.conf";
    let contents = fs::read_to_string(path).context("failed to read /etc/pacman.conf")?;
    let sections = parse_pacman_conf(&contents);

    // "options" and the pre-amble (empty header) never move — only actual
    // repo sections get reordered.
    let repo_sections: Vec<&ConfSection> = sections
        .iter()
        .filter(|s| !s.header.is_empty() && section_name(&s.header) != "options")
        .collect();

    let priorities: Vec<u8> = repo_sections
        .iter()
        .map(|s| section_priority(&section_name(&s.header)))
        .collect();

    let already_sorted = priorities.windows(2).all(|w| w[0] <= w[1]);

    if already_sorted {
        return Ok(CheckStatus::Ok(
            "Artix's own repos are already prioritized above CachyOS in pacman.conf".to_string(),
        ));
    }

    let problem =
        "pacman.conf has a CachyOS repo listed above one of Artix's own repos — this risks \
         pulling systemd-linked packages onto a dinit system"
            .to_string();

    if !apply {
        return Ok(CheckStatus::NeedsAttention(problem));
    }

    // Back up before touching anything.
    fs::copy(path, format!("{path}.distro-tool.bak"))
        .context("failed to back up pacman.conf before editing")?;

    // Rebuild: preamble + options untouched, then repo sections stable-sorted.
    let mut new_contents = String::new();
    for s in sections.iter().filter(|s| {
        s.header.is_empty() || section_name(&s.header) == "options"
    }) {
        if !s.header.is_empty() {
            new_contents.push_str(&s.header);
            new_contents.push('\n');
        }
        for line in &s.body {
            new_contents.push_str(line);
            new_contents.push('\n');
        }
    }

    let mut sorted_repos: Vec<&ConfSection> = repo_sections;
    sorted_repos.sort_by_key(|s| section_priority(&section_name(&s.header)));

    for s in sorted_repos {
        new_contents.push_str(&s.header);
        new_contents.push('\n');
        for line in &s.body {
            new_contents.push_str(line);
            new_contents.push('\n');
        }
    }

    // Write via a temp file + sudo mv, since pacman.conf isn't user-writable.
    let tmp_path = "/tmp/distro-tool-pacman.conf";
    fs::write(tmp_path, &new_contents).context("failed to write temp pacman.conf")?;
    let mv = run("sudo", &["mv", tmp_path, path])?;
    if !mv.status.success() {
        return Ok(CheckStatus::NeedsAttention(format!(
            "{problem} — could not write the fix: {}",
            String::from_utf8_lossy(&mv.stderr).trim()
        )));
    }

    // Refresh package databases so the new priority actually takes effect.
    let _ = run("sudo", &["pacman", "-Syy"]);

    Ok(CheckStatus::Fixed(format!(
        "{problem} — reordered so Artix's repos come first (backup saved as \
         pacman.conf.distro-tool.bak)"
    )))
}

// -------------------------------------------------------
// Check 3: linux-firmware is installed.
//
// Without it, amdgpu (and most other GPU/wifi/etc. drivers) fail to
// initialize at all — this showed up as "only one display works" on
// RDNA4 hardware, but the root cause is generic and affects any
// firmware-dependent device.
// -------------------------------------------------------
pub fn check_firmware(apply: bool) -> Result<CheckStatus> {
    if command_succeeds("pacman", &["-Qi", "linux-firmware"]) {
        return Ok(CheckStatus::Ok("linux-firmware is installed".to_string()));
    }

    let problem =
        "linux-firmware is not installed — GPU, wifi, and other hardware may fail to \
         initialize properly"
            .to_string();

    if !apply {
        return Ok(CheckStatus::NeedsAttention(problem));
    }

    let install = run("sudo", &["pacman", "-S", "--needed", "--noconfirm", "linux-firmware"])?;
    if install.status.success() {
        Ok(CheckStatus::Fixed(format!(
            "{problem} — installed now. A reboot is needed for it to take effect."
        )))
    } else {
        Ok(CheckStatus::NeedsAttention(format!(
            "{problem} — install attempt failed: {}",
            String::from_utf8_lossy(&install.stderr).trim()
        )))
    }
}

pub fn all_checks() -> Vec<(&'static str, fn(bool) -> Result<CheckStatus>)> {
    vec![
        ("Login shell", check_shell),
        ("Repo priority", check_repo_order),
        ("GPU/hardware firmware", check_firmware),
    ]
}

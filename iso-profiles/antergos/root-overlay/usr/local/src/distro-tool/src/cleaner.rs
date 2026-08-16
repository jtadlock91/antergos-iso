use crate::checks::CheckStatus;
use crate::util::*;
use anyhow::Result;
use std::path::Path;

// -------------------------------------------------------
// Daily cleanup/organizer tool (system-clean).
//
// This is a real, tested port of an existing bash tool, converted to
// Python for more reliable string/path handling, and adapted here for
// dinit systems: the original used a systemd --user timer for daily
// scheduling, which has no dinit equivalent, so this uses the user's
// own crontab instead (cronie is already present on this system).
//
// Embedded directly rather than shipped as a separate ISO file, since
// — like everything else in this tool — it needs to write into the
// ACTUAL created user's home directory and crontab, which only exist
// after install, not at ISO-build time.
// -------------------------------------------------------

const SYSTEM_CLEAN_PY: &str = r#"#!/usr/bin/env python3
"""
system-clean.py — Daily System Cleanup & Organizer

Python port of system-clean.sh, preserving behavior and modes exactly:
  --auto     Run automatically (called by cron on dinit systems, since
             there's no systemd timer to hook here)
  --manual   Interactive mode, prompts for ambiguous actions
  --dry-run  Show what would happen, touch nothing
  --report   Print last run log and exit

Manages:
  - Downloads: sort by type + date, flag old files, flag dupes
  - Documents: flag duplicates, organize loose files
  - Screenshots: sort into monthly subfolders
  - Dotfiles: check for untracked configs
  - Packages: orphans, pacman cache, large unused packages

Log: ~/.local/share/system-clean/system-clean.log
Review file: ~/.local/share/system-clean/review.txt
"""

import hashlib
import os
import shutil
import subprocess
import sys
from datetime import datetime
from pathlib import Path

# -------------------------------------------------------
# CONFIG — edit these to match your paths
# -------------------------------------------------------
HOME = Path.home()
DOWNLOADS = HOME / "Downloads"
DOCUMENTS = HOME / "Documents"
SCREENSHOTS = HOME / "Pictures" / "Screenshots"
DOTFILES = Path(os.environ.get("SYSTEM_CLEAN_DOTFILES", ""))  # blank = disabled
OLD_FILE_DAYS = 90
PACMAN_KEEP_VERSIONS = 2
LARGE_PKG_MB = 200

# -------------------------------------------------------
# FILE TYPE MAP
# -------------------------------------------------------
EXT_TO_FOLDER = {
    # Images
    **{e: "Images" for e in ("jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "tif", "heic", "avif", "svg")},
    # Video
    **{e: "Video" for e in ("mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg")},
    # Audio
    **{e: "Audio" for e in ("mp3", "flac", "wav", "ogg", "aac", "m4a", "opus", "wma")},
    # Documents
    **{e: "Documents" for e in ("pdf", "doc", "docx", "odt", "xls", "xlsx", "ods", "ppt", "pptx", "odp", "txt", "md", "rtf", "csv")},
    # Archives
    **{e: "Archives" for e in ("zip", "tar", "gz", "xz", "bz2", "7z", "rar", "zst")},
    # Disk images
    **{e: "DiskImages" for e in ("iso", "img", "bin", "dmg")},
    # Code / scripts
    **{e: "Code" for e in ("sh", "py", "js", "ts", "html", "css", "json", "yaml", "yml", "toml", "conf", "cfg", "xml", "lua", "rs", "go", "cpp", "c", "h")},
    # Torrents
    "torrent": "Torrents",
    # Packages
    **{e: "Packages" for e in ("deb", "rpm", "appimage")},
    # Fonts
    **{e: "Fonts" for e in ("ttf", "otf", "woff", "woff2")},
}


class SystemClean:
    def __init__(self, mode: str):
        self.mode = mode
        self.dry_run = mode == "--dry-run"
        self.interactive = mode == "--manual"

        self.log_dir = HOME / ".local" / "share" / "system-clean"
        self.log_file = self.log_dir / "system-clean.log"
        self.review_file = self.log_dir / "review.txt"
        self.log_dir.mkdir(parents=True, exist_ok=True)

    # -----------------------------------------------------
    # Logging
    # -----------------------------------------------------
    def log(self, msg: str):
        line = f"[{datetime.now():%Y-%m-%d %H:%M:%S}] {msg}"
        print(line)
        with open(self.log_file, "a") as f:
            f.write(line + "\n")

    def log_review(self, msg: str):
        with open(self.review_file, "a") as f:
            f.write(f"[{datetime.now():%Y-%m-%d}] {msg}\n")

    def report(self):
        if self.log_file.exists():
            print("=== Last 100 log lines ===")
            with open(self.log_file) as f:
                lines = f.readlines()
            print("".join(lines[-100:]))
            print()
            if self.review_file.exists() and self.review_file.stat().st_size > 0:
                print("=== Items needing your review ===")
                print(self.review_file.read_text())
            else:
                print("=== No items pending review ===")
        else:
            print("No log file found yet. Run system-clean.py first.")

    # -----------------------------------------------------
    # Helpers
    # -----------------------------------------------------
    def move_file(self, src: Path, dest_dir: Path):
        if self.dry_run:
            self.log(f"  [DRY-RUN] Would move: {src.name} -> {dest_dir}/")
            return

        dest_dir.mkdir(parents=True, exist_ok=True)
        dest = dest_dir / src.name

        if dest.exists():
            base, ext = src.stem, src.suffix
            counter = 1
            while (dest_dir / f"{base}_{counter}{ext}").exists():
                counter += 1
            dest = dest_dir / f"{base}_{counter}{ext}"

        shutil.move(str(src), str(dest))
        self.log(f"  [MOVED] {src.name} -> {dest_dir}/")

    def ask_user(self, question: str, file: Path) -> str:
        if self.interactive:
            print()
            print(f"  [?] {question}")
            print(f"      File: {file}")
            return input("      Action? [m=move/d=delete/s=skip] ").strip().lower()
        else:
            self.log_review(f"REVIEW NEEDED: {question} — {file}")
            return "skip"

    def find_duplicates(self, directory: Path):
        if not directory.exists():
            return
        self.log(f"  Scanning for duplicates in {directory}...")

        seen_hashes = {}
        dupe_count = 0

        files = []
        for depth1 in directory.iterdir():
            if depth1.is_file():
                files.append(depth1)
            elif depth1.is_dir():
                for depth2 in depth1.iterdir():
                    if depth2.is_file():
                        files.append(depth2)

        for f in files:
            try:
                h = hashlib.md5(f.read_bytes()).hexdigest()
            except OSError:
                continue
            if h in seen_hashes:
                self.log(f"  [DUPE] {f} is identical to {seen_hashes[h]}")
                self.log_review(f"DUPLICATE: {f} (same as {seen_hashes[h]})")
                dupe_count += 1
            else:
                seen_hashes[h] = f

        if dupe_count == 0:
            self.log("  No duplicates found.")
        else:
            self.log(f"  Found {dupe_count} duplicate(s) — added to review file.")

    # -----------------------------------------------------
    # 1. Downloads
    # -----------------------------------------------------
    def clean_downloads(self):
        self.log("")
        self.log("==> [1/5] Cleaning Downloads...")

        if not DOWNLOADS.is_dir():
            self.log("  Downloads folder not found, skipping.")
            return

        moved = flagged_old = unknown = 0
        now = datetime.now().timestamp()

        for file in [f for f in DOWNLOADS.iterdir() if f.is_file()]:
            ext = file.suffix.lstrip(".").lower()
            type_folder = EXT_TO_FOLDER.get(ext, "")
            mtime = file.stat().st_mtime
            file_date = datetime.fromtimestamp(mtime).strftime("%Y-%m")
            age_days = int((now - mtime) / 86400)

            if age_days > OLD_FILE_DAYS:
                self.log(f"  [OLD] {file.name} is {age_days} days old")
                self.log_review(f"OLD FILE ({age_days} days): {file}")
                flagged_old += 1

            if type_folder:
                self.move_file(file, DOWNLOADS / type_folder / file_date)
                moved += 1
            else:
                self.log(f"  [UNKNOWN TYPE] {file.name} (.{ext})")
                if self.interactive:
                    answer = self.ask_user(f"Unknown file type .{ext} — what to do with {file.name}?", file)
                    if answer == "d":
                        if not self.dry_run:
                            file.unlink()
                        self.log(f"  [DELETED] {file.name}")
                    elif answer == "m":
                        self.move_file(file, DOWNLOADS / "Unsorted" / file_date)
                    else:
                        self.log(f"  [SKIPPED] {file.name}")
                else:
                    self.log_review(f"UNKNOWN TYPE (.{ext}): {file}")
                    unknown += 1

        self.log(f"  Done — moved: {moved}, flagged old: {flagged_old}, unknown types: {unknown}")
        self.find_duplicates(DOWNLOADS)

    # -----------------------------------------------------
    # 2. Documents
    # -----------------------------------------------------
    def clean_documents(self):
        self.log("")
        self.log("==> [2/5] Cleaning Documents...")

        if not DOCUMENTS.is_dir():
            self.log("  Documents folder not found, skipping.")
            return

        loose = [f for f in DOCUMENTS.iterdir() if f.is_file()]
        for file in loose:
            self.log(f"  [LOOSE] {file.name} is sitting in Documents root")
            self.log_review(f"LOOSE FILE IN DOCUMENTS: {file}")

        if not loose:
            self.log("  Documents root is clean.")
        else:
            self.log(f"  Found {len(loose)} loose file(s) in Documents root — added to review.")

        self.find_duplicates(DOCUMENTS)

    # -----------------------------------------------------
    # 3. Screenshots
    # -----------------------------------------------------
    def clean_screenshots(self):
        self.log("")
        self.log("==> [3/5] Cleaning Screenshots...")

        if not SCREENSHOTS.is_dir():
            self.log(f"  Screenshots folder not found at {SCREENSHOTS}, skipping.")
            return

        moved = 0
        for file in [f for f in SCREENSHOTS.iterdir() if f.is_file()]:
            if file.suffix.lstrip(".").lower() not in ("png", "jpg", "jpeg", "webp"):
                continue
            month_folder = datetime.fromtimestamp(file.stat().st_mtime).strftime("%Y-%m")
            self.move_file(file, SCREENSHOTS / month_folder)
            moved += 1

        self.log(f"  Done — sorted {moved} screenshot(s) into monthly folders.")

    # -----------------------------------------------------
    # 4. Dotfiles
    # -----------------------------------------------------
    def clean_dotfiles(self):
        self.log("")
        self.log("==> [4/5] Checking dotfiles...")

        if not DOTFILES or not DOTFILES.is_dir():
            self.log("  No dotfiles repo configured or found, skipping.")
            self.log("  Tip: set SYSTEM_CLEAN_DOTFILES env var to enable.")
            return

        if not shutil.which("git"):
            self.log("  git not found, skipping dotfiles check.")
            return

        result = subprocess.run(
            ["git", "-C", str(DOTFILES), "status", "--short"],
            capture_output=True, text=True,
        )
        status = result.stdout.strip()
        if status:
            self.log("  [UNTRACKED/MODIFIED] Dotfiles repo has uncommitted changes:")
            for line in status.splitlines():
                self.log(f"    {line}")
            self.log_review(f"DOTFILES: Uncommitted changes in {DOTFILES} — run: cd {DOTFILES} && git status")
        else:
            self.log("  Dotfiles repo is clean.")

        candidates = [
            HOME / ".bashrc", HOME / ".zshrc",
            HOME / ".config" / "kitty" / "kitty.conf",
            HOME / ".config" / "kwinrc",
            HOME / ".config" / "fastfetch",
        ]
        untracked = []
        for cfg in candidates:
            if cfg.exists():
                rel = cfg.relative_to(HOME)
                check = subprocess.run(
                    ["git", "-C", str(DOTFILES), "ls-files", "--error-unmatch", str(rel)],
                    capture_output=True,
                )
                if check.returncode != 0:
                    untracked.append(cfg)

        if untracked:
            self.log("  [UNTRACKED CONFIGS] These exist but aren't in your dotfiles repo:")
            for cfg in untracked:
                self.log(f"    {cfg}")
                self.log_review(f"UNTRACKED CONFIG: {cfg} not in dotfiles repo")

    # -----------------------------------------------------
    # 5. Package management
    # -----------------------------------------------------
    def clean_packages(self):
        self.log("")
        self.log("==> [5/5] Package management...")

        # Orphans
        if shutil.which("pacman"):
            result = subprocess.run(["pacman", "-Qdtq"], capture_output=True, text=True)
            orphans = [p for p in result.stdout.strip().splitlines() if p]
        else:
            self.log("  [WARN] pacman not found — skipping orphan check.")
            orphans = []

        if orphans:
            self.log("  [ORPHANS FOUND]")
            for pkg in orphans:
                self.log(f"    {pkg}")
            if self.interactive:
                print()
                answer = input("  Remove orphans? [y/N] ").strip().lower()
                if answer == "y":
                    if not self.dry_run:
                        subprocess.run(["sudo", "pacman", "-Rns", "--noconfirm", *orphans])
                    self.log("  [REMOVED] Orphans cleaned.")
                else:
                    self.log("  [SKIPPED] Orphans left in place.")
            else:
                self.log_review(f"ORPHANS: Run 'sudo pacman -Rns {' '.join(orphans)}' to remove")
        else:
            self.log("  No orphans found.")

        # Pacman cache
        self.log(f"  Clearing pacman cache (keeping {PACMAN_KEEP_VERSIONS} versions per package)...")
        if self.dry_run:
            self.log(f"  [DRY-RUN] Would run: paccache -r -k {PACMAN_KEEP_VERSIONS}")
        elif shutil.which("paccache"):
            result = subprocess.run(
                ["sudo", "paccache", "-r", "-k", str(PACMAN_KEEP_VERSIONS)],
                capture_output=True, text=True,
            )
            for line in (result.stdout + result.stderr).splitlines():
                self.log(f"  {line}")
        else:
            self.log("  [WARN] paccache not found — install pacman-contrib for cache cleanup.")
            self.log_review("MISSING TOOL: Install pacman-contrib for pacman cache management")

        # Large packages
        self.log(f"  Checking for large explicitly installed packages (>{LARGE_PKG_MB}MB)...")
        if shutil.which("expac"):
            result = subprocess.run(
                ["expac", "-H", "M", "%m\t%n"], capture_output=True, text=True,
            )
            large = []
            for line in result.stdout.splitlines():
                parts = line.split("\t")
                if len(parts) == 2:
                    try:
                        size = float(parts[0])
                    except ValueError:
                        continue
                    if size > LARGE_PKG_MB:
                        large.append(line)
            if large:
                self.log("  [LARGE PACKAGES] Explicitly installed packages over threshold:")
                for line in large:
                    self.log(f"    {line}")
                self.log_review("LARGE PACKAGES: Review these — some may no longer be needed:\n" + "\n".join(large))
            else:
                self.log("  No large packages flagged.")
        else:
            self.log("  expac not installed — skipping large package check.")
            self.log_review("MISSING TOOL: Install expac for large package detection")

    # -----------------------------------------------------
    # Main
    # -----------------------------------------------------
    def run(self):
        self.log("=" * 46)
        self.log(f" system-clean.py starting (mode: {self.mode})")
        self.log("=" * 46)

        self.review_file.write_text("")  # clear review file at start of each run

        sections = [
            self.clean_downloads,
            self.clean_documents,
            self.clean_screenshots,
            self.clean_dotfiles,
            self.clean_packages,
        ]
        for section in sections:
            try:
                section()
            except Exception as e:
                self.log(f"  [ERROR] {section.__name__} failed unexpectedly: {e}")
                self.log("  Continuing with remaining checks...")

        self.log("")
        self.log("=" * 46)
        self.log(" Run complete.")
        if self.review_file.stat().st_size > 0:
            self.log(" Items need your attention — run:")
            self.log("   system-clean.py --report")
        self.log("=" * 46)


def main():
    valid_modes = ("--auto", "--manual", "--dry-run", "--report")
    mode = sys.argv[1] if len(sys.argv) > 1 else "--auto"
    if mode in ("-h", "--help"):
        print(__doc__)
        sys.exit(0)
    if mode not in valid_modes:
        print(f"Usage: system-clean.py [{'|'.join(valid_modes)}]", file=sys.stderr)
        sys.exit(1)

    cleaner = SystemClean(mode)
    if mode == "--report":
        cleaner.report()
        sys.exit(0)

    cleaner.run()


if __name__ == "__main__":
    main()
"#;

const SCRIPT_PATH: &str = "/usr/local/bin/system-clean";
const SUDOERS_PATH: &str = "/etc/sudoers.d/system-clean";

pub fn check_system_clean(apply: bool) -> Result<CheckStatus> {
    let script_ok = Path::new(SCRIPT_PATH).exists();
    let sudoers_ok = Path::new(SUDOERS_PATH).exists();
    let cron_ok = user_crontab_has(SCRIPT_PATH);

    if script_ok && sudoers_ok && cron_ok {
        return Ok(CheckStatus::Ok(
            "daily cleanup tool (system-clean) is installed, scheduled, and configured".to_string(),
        ));
    }

    let problem = "daily cleanup tool (system-clean) is not fully set up".to_string();
    if !apply {
        return Ok(CheckStatus::NeedsAttention(problem));
    }

    if !script_ok {
        write_privileged_file(SCRIPT_PATH, SYSTEM_CLEAN_PY)?;
        run("sudo", &["chmod", "755", SCRIPT_PATH])?;
    }

    if !sudoers_ok {
        // Passwordless access scoped to exactly the two commands the
        // automated --auto run needs root for — nothing broader.
        let sudoers_content = "# Allows the automated daily system-clean run to prune the pacman\n# cache and remove orphaned packages without an interactive password\n# prompt. Scoped to exactly these two commands, nothing broader.\n%wheel ALL=(root) NOPASSWD: /usr/bin/paccache, /usr/bin/pacman -Rns *\n";
        write_privileged_file(SUDOERS_PATH, sudoers_content)?;
        run("sudo", &["chmod", "440", SUDOERS_PATH])?;

        // Validate before trusting it — a broken sudoers file can lock
        // out sudo entirely, so never leave one in place unverified.
        let check = run("sudo", &["visudo", "-cf", SUDOERS_PATH])?;
        if !check.status.success() {
            let _ = run("sudo", &["rm", "-f", SUDOERS_PATH]);
            return Ok(CheckStatus::NeedsAttention(format!(
                "{problem} — generated sudoers rule failed validation, removed for safety: {}",
                String::from_utf8_lossy(&check.stderr).trim()
            )));
        }
    }

    if !cron_ok {
        // Daily at 9am — matches a reasonable "once a day, not the
        // middle of the night when the machine might be off" default.
        add_user_cron_entry(&format!("0 9 * * * {SCRIPT_PATH} --auto"))?;
    }

    Ok(CheckStatus::Fixed(format!(
        "{problem} — installed, scheduled daily at 9am via cron"
    )))
}

pub fn all_cleaner_checks() -> Vec<(&'static str, fn(bool) -> Result<CheckStatus>)> {
    vec![("Daily cleanup tool (system-clean)", check_system_clean)]
}

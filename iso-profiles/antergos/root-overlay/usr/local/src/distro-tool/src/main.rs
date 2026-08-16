mod checks;
mod cleaner;
mod gpu;
mod snapshots;
mod tuning;
mod util;

use anyhow::Result;
use checks::{all_checks, CheckStatus};
use clap::{Parser, Subcommand};
use util::run;

#[derive(Parser)]
#[command(
    name = "distro-tool",
    about = "Keeps this system healthy. Safe to run any time — it only ever \
             checks and fixes known issues, never touches anything else."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check and fix known issues, including background performance
    /// tuning. Safe to run repeatedly.
    Setup,
    /// Report on known issues without changing anything.
    Doctor,
    /// (Coming soon) Optional gaming setup: Steam, GameMode, Proton.
    Gaming,
    /// Re-run just the background performance tuning checks on their own.
    Tune,
    /// Kernel-related choices. Nothing here runs automatically.
    Kernel {
        #[command(subcommand)]
        action: KernelAction,
    },
    /// Nvidia driver setup guidance. Detects your hardware and prints
    /// what to run — never installs anything automatically, since
    /// driver choice needs a deliberate decision.
    Gpu,
}

#[derive(Subcommand)]
enum KernelAction {
    /// Install the CachyOS RC kernel alongside your current one. Opt-in —
    /// only worth it if you have hardware needing very recent kernel
    /// support (e.g. brand-new GPUs). You'll need to select it manually
    /// from the boot menu; your current kernel stays as the default.
    Rc,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Setup => run_checks(true),
        Commands::Doctor => run_checks(false),
        Commands::Gaming => {
            println!("Gaming setup isn't built yet — coming in a future update.");
            println!("For now, Steam is entirely optional and never installed automatically.");
            Ok(())
        }
        Commands::Tune => run_check_set("Re-checking performance tuning...", tuning::all_tuning_checks(), true),
        Commands::Kernel { action } => match action {
            KernelAction::Rc => install_rc_kernel(),
        },
        Commands::Gpu => gpu::print_nvidia_guidance(),
    }
}

fn run_checks(apply: bool) -> Result<()> {
    let mut all = all_checks();
    all.extend(tuning::all_tuning_checks());
    all.extend(snapshots::all_snapshot_checks());
    all.extend(cleaner::all_cleaner_checks());
    all.extend(gpu::all_gpu_checks());

    let header = if apply {
        "Checking your system and fixing anything that needs it..."
    } else {
        "Checking your system (nothing will be changed)..."
    };
    run_check_set(header, all, apply)
}

fn run_check_set(
    header: &str,
    checks: Vec<(&'static str, fn(bool) -> Result<CheckStatus>)>,
    apply: bool,
) -> Result<()> {
    println!("{header}\n");

    let mut fixed = 0;
    let mut ok = 0;
    let mut attention = 0;

    for (name, check_fn) in checks {
        match check_fn(apply) {
            Ok(status) => {
                println!("[{}]  {name}: {}", status.label(), status.message());
                match status {
                    CheckStatus::Ok(_) => ok += 1,
                    CheckStatus::Fixed(_) => fixed += 1,
                    CheckStatus::NeedsAttention(_) => attention += 1,
                }
            }
            Err(e) => {
                // A check erroring out (e.g. a command genuinely missing)
                // never stops the rest of the run — every other check
                // still gets a fair chance to run and report.
                println!("? {name}: couldn't complete this check — {e}");
                attention += 1;
            }
        }
    }

    println!();
    if attention == 0 {
        println!("Everything looks good! ({ok} already fine, {fixed} fixed just now)");
    } else {
        println!(
            "{ok} already fine, {fixed} fixed, {attention} need a closer look — see above."
        );
        if !apply {
            println!("Run `distro-tool setup` to fix what can be fixed automatically.");
        }
    }

    Ok(())
}

fn install_rc_kernel() -> Result<()> {
    println!("Installing the CachyOS RC kernel alongside your current one.");
    println!("This is entirely optional and your current kernel stays as the default —");
    println!("you'll pick the RC kernel manually from the boot menu when you want it.\n");

    let install = run(
        "sudo",
        &["pacman", "-S", "--needed", "--noconfirm", "linux-cachyos-rc", "linux-cachyos-rc-headers"],
    )?;
    if !install.status.success() {
        println!(
            "Install failed: {}",
            String::from_utf8_lossy(&install.stderr).trim()
        );
        return Ok(());
    }

    let grub = run("sudo", &["grub-mkconfig", "-o", "/boot/grub/grub.cfg"])?;
    if !grub.status.success() {
        println!(
            "Kernel installed, but updating the boot menu failed: {}",
            String::from_utf8_lossy(&grub.stderr).trim()
        );
        println!("You may need to run this manually: sudo grub-mkconfig -o /boot/grub/grub.cfg");
        return Ok(());
    }

    println!("Done! The RC kernel is now available at your next reboot.");
    println!("At the boot menu, look for an entry mentioning \"cachyos-rc\" and select it —");
    println!("your normal kernel will still boot by default if you don't.");
    Ok(())
}

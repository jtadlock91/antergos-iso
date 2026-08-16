use crate::checks::CheckStatus;
use crate::util::*;
use anyhow::Result;
use std::fs;

// -------------------------------------------------------
// GPU detection and vendor-appropriate tuning.
//
// AMD and Intel are auto-tuned by `setup` — both drivers are
// open-source, already loaded by the kernel, and applying the right
// flags/packages carries essentially no risk of breaking the display.
//
// Nvidia is deliberately NOT auto-installed, even under `setup --apply`.
// Picking a driver means choosing between nvidia-dkms/nvidia,
// open/proprietary, and possibly PRIME hybrid config — getting that
// wrong unattended on a first boot risks a black screen, the worst
// possible outcome for something meant to "just work". Nvidia users
// get clear guidance and run `distro-tool gpu` themselves once they've
// read it.
// -------------------------------------------------------

#[derive(Debug, PartialEq)]
pub enum GpuVendor {
    Amd,
    Intel,
    Nvidia,
    NvidiaHybrid, // Nvidia + Intel, e.g. laptop PRIME setups
    Unknown,
}

pub fn detect_gpu_vendor() -> Result<GpuVendor> {
    let lspci_out = run("lspci", &[])?;
    let lspci_text = String::from_utf8_lossy(&lspci_out.stdout);

    let display_lines: String = lspci_text
        .lines()
        .filter(|l| {
            let lower = l.to_lowercase();
            lower.contains("vga") || lower.contains("3d") || lower.contains("display")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();

    // Deliberately no bare "ati" check — it collides with the extremely
    // common word "Corporation" (found in nearly every lspci vendor
    // string), which caused every GPU to falsely detect as AMD during
    // testing. "AMD" and "Radeon" alone are specific enough; modern
    // lspci output always includes at least one of them for AMD/ATI
    // hardware.
    let has_amd = display_lines.contains("amd") || display_lines.contains("radeon");
    let has_nvidia = display_lines.contains("nvidia");
    let has_intel = display_lines.contains("intel");

    Ok(if has_amd && !has_nvidia {
        GpuVendor::Amd
    } else if has_nvidia && has_intel && !has_amd {
        GpuVendor::NvidiaHybrid
    } else if has_nvidia && !has_intel && !has_amd {
        GpuVendor::Nvidia
    } else if has_intel && !has_nvidia && !has_amd {
        GpuVendor::Intel
    } else if has_amd && has_nvidia {
        // AMD + Nvidia hybrid is rare; treat as AMD-primary since that's
        // the safe, open-source driver to auto-tune. Nvidia guidance
        // still applies separately if they want the discrete card too.
        GpuVendor::Amd
    } else {
        GpuVendor::Unknown
    })
}

fn amd_tuning_applied() -> bool {
    let env_ok = fs::read_to_string("/etc/environment")
        .map(|c| c.contains("RADV_PERFTEST"))
        .unwrap_or(false);
    let udev_ok = std::path::Path::new("/etc/udev/rules.d/30-amdgpu-pm.rules").exists();
    env_ok && udev_ok
}

fn apply_amd_tuning() -> Result<()> {
    let env_contents = fs::read_to_string("/etc/environment").unwrap_or_default();
    let mut new_env = env_contents.clone();
    if !new_env.contains("RADV_PERFTEST") {
        if !new_env.is_empty() && !new_env.ends_with('\n') {
            new_env.push('\n');
        }
        new_env.push_str("RADV_PERFTEST=gpl,nggc\n");
    }
    if !new_env.contains("mesa_glthread") {
        new_env.push_str("mesa_glthread=true\n");
    }
    if new_env != env_contents {
        write_privileged_file("/etc/environment", &new_env)?;
    }

    let udev_rule = "ACTION==\"add\", SUBSYSTEM==\"drm\", KERNEL==\"card*\", \\\n  ATTR{device/power_dpm_force_performance_level}=\"high\"\n";
    write_privileged_file("/etc/udev/rules.d/30-amdgpu-pm.rules", udev_rule)?;
    let _ = run("sudo", &["udevadm", "control", "--reload-rules"]);
    Ok(())
}

fn intel_tuning_applied() -> bool {
    package_installed("vulkan-intel") && package_installed("intel-media-driver")
}

fn apply_intel_tuning() -> Result<()> {
    let install = run(
        "sudo",
        &["pacman", "-S", "--needed", "--noconfirm", "vulkan-intel", "intel-media-driver"],
    )?;
    if !install.status.success() {
        anyhow::bail!(
            "install failed: {}",
            String::from_utf8_lossy(&install.stderr).trim()
        );
    }
    Ok(())
}

pub fn check_gpu(apply: bool) -> Result<CheckStatus> {
    let vendor = detect_gpu_vendor()?;

    match vendor {
        GpuVendor::Amd => {
            if amd_tuning_applied() {
                return Ok(CheckStatus::Ok("AMD GPU detected — tuning already applied".to_string()));
            }
            let problem = "AMD GPU detected — Vulkan/performance tuning not yet applied".to_string();
            if !apply {
                return Ok(CheckStatus::NeedsAttention(problem));
            }
            match apply_amd_tuning() {
                Ok(_) => Ok(CheckStatus::Fixed(format!(
                    "{problem} — applied (takes full effect after next login)"
                ))),
                Err(e) => Ok(CheckStatus::NeedsAttention(format!("{problem} — {e}"))),
            }
        }
        GpuVendor::Intel => {
            if intel_tuning_applied() {
                return Ok(CheckStatus::Ok(
                    "Intel GPU detected — driver packages already installed".to_string(),
                ));
            }
            let problem = "Intel GPU detected — Vulkan/hardware-decode packages not yet installed".to_string();
            if !apply {
                return Ok(CheckStatus::NeedsAttention(problem));
            }
            match apply_intel_tuning() {
                Ok(_) => Ok(CheckStatus::Fixed(format!("{problem} — installed"))),
                Err(e) => Ok(CheckStatus::NeedsAttention(format!("{problem} — {e}"))),
            }
        }
        GpuVendor::Nvidia | GpuVendor::NvidiaHybrid => {
            // Deliberately the same message whether apply is true or
            // false — this is the one case where `setup` should never
            // silently act, even when told to fix things automatically.
            Ok(CheckStatus::NeedsAttention(
                "Nvidia GPU detected — driver setup needs a deliberate choice, not an \
                 automatic install. Run `distro-tool gpu` for guidance."
                    .to_string(),
            ))
        }
        GpuVendor::Unknown => Ok(CheckStatus::Ok(
            "No GPU vendor confidently detected — skipping GPU-specific tuning".to_string(),
        )),
    }
}

/// Print Nvidia driver guidance — informational only, never executes
/// anything itself. The person reviews and runs these commands
/// themselves once they've decided.
pub fn print_nvidia_guidance() -> Result<()> {
    let vendor = detect_gpu_vendor()?;
    if vendor != GpuVendor::Nvidia && vendor != GpuVendor::NvidiaHybrid {
        println!("No Nvidia GPU detected on this system — nothing to do here.");
        return Ok(());
    }

    let kernel_out = run("uname", &["-r"])?;
    let kernel = String::from_utf8_lossy(&kernel_out.stdout).trim().to_string();
    let needs_dkms = kernel.contains("cachyos") || kernel.contains("zen") || kernel.contains("tkg");

    println!("Nvidia GPU detected{}.\n",
        if vendor == GpuVendor::NvidiaHybrid { " (hybrid — Intel + Nvidia)" } else { "" }
    );
    println!("This needs a deliberate choice, so nothing has been installed automatically.");
    println!("Current kernel: {kernel}\n");

    if needs_dkms {
        println!("Your kernel isn't the standard Artix one, so you need the DKMS driver");
        println!("build (compiles against your specific kernel) rather than the prebuilt one:\n");
        println!("  sudo pacman -S nvidia-dkms nvidia-utils lib32-nvidia-utils opencl-nvidia nvidia-settings\n");
    } else {
        println!("You're on the standard kernel, so the prebuilt driver works directly:\n");
        println!("  sudo pacman -S nvidia nvidia-utils lib32-nvidia-utils opencl-nvidia nvidia-settings\n");
    }

    println!("Add nvidia_drm.modeset=1 to your kernel command line afterward (needed for");
    println!("Wayland sessions to work correctly with the Nvidia driver).\n");

    if vendor == GpuVendor::NvidiaHybrid {
        println!("Since this is a hybrid laptop setup, you'll also want envycontrol to");
        println!("switch between integrated and discrete GPU modes (from AUR, via paru):\n");
        println!("  paru -S envycontrol");
        println!("  sudo envycontrol -s hybrid\n");
    }

    println!("Reboot after installing for the driver to actually take effect.");
    Ok(())
}

pub fn all_gpu_checks() -> Vec<(&'static str, fn(bool) -> Result<CheckStatus>)> {
    vec![("GPU driver tuning", check_gpu)]
}

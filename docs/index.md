---
title: Home
layout: home
nav_order: 1
---

# Antergos NeXT

A community revival of Antergos for the post-systemd era — **Artix Linux** base with **Dinit**, **KDE Plasma**, and the **Calamares** installer.

## Download

[**Download the latest ISO**](https://github.com/Antergos-NeXT/antergos-iso/releases)
_Published to GitHub Releases. (The Internet Archive upload step in CI is currently disabled — see [CI](ci).)_

### Latest release — v2026.07.11

What works in the latest release:

- KDE Plasma 6 on Wayland (with SDDM)
- Full audio support on installed systems
- Custom SDDM theme (not Breeze)
- Correct `/usr/lib/os-release` (shows "Antergos NeXT", not "Artix Linux")
- Choose your desktop (Plasma/Xfce/Cinnamon/MATE/LXQt/i3/Sway/Hyprland/COSMIC)
- GRUB with Antergos theme
- Custom Calamares slideshow
- Xlibre X server included

> Note: the current `master` branch moves the SDDM theme to `pixie` (`pixie-sddm-git`). The v2026.07.11 release used the older theme — see the [releases page](https://github.com/Antergos-NeXT/antergos-iso/releases) for per-release changes.

The offline bare-minimum installer is **experimental and best-effort** — the online Calamares flow is the supported path.

## Quick links

- [Building the ISO](building) — set up and build locally
- [Installer](installer) — Calamares modes, init & DE selectors
- [Custom packages](packages) — PKGBUILDs, pipewire fork, branding
- [CI/CD](ci) — GitHub Actions pipeline, Internet Archive upload
- [Development](development) — contributing, gotchas, conventions

## Learn about Antergos NeXT

- [Init Systems](init-systems) — Dinit, OpenRC, S6, Runit compared
- [Desktop Environments](desktop-environments) — available DEs in online mode
- [Wallpapers](wallpapers) — where they go, how they work
- [Restoring Languages](restoring-languages) — this fork ships English-only by default; how to add a language back
- [Offline Installer](byode) — the BYODE bare-minimum installer

## What changed from original Antergos

| Area | Original Antergos | Antergos NeXT |
|------|-------------------|---------------|
| Base | Arch Linux (systemd) | Artix Linux |
| Default init | systemd | Dinit (others via [changing-init](changing-init)) |
| Desktop | GNOME | KDE Plasma |
| Installer | Custom Cnchi | Calamares (online) + BYODE (offline) |
| Build system | archiso | artools (`buildiso`) |
| Display server | X11 | Wayland (X11 via Xlibre) |

## Project scope

This repo (`antergos-iso`) contains the ISO build configuration — overlays, Calamares modules, pacman config, CI pipeline. Custom PKGBUILDs live in the separate [antergos-packages](https://github.com/Antergos-NeXT/antergos-packages) repo.

## Sources

- [Artix Linux](https://artixlinux.org)
- [artools](https://gitea.artixlinux.org/artix/artools)
- [Calamares](https://codeberg.org/calamares/calamares)
- [antergos-packages](https://github.com/Antergos-NeXT/antergos-packages)
- [Original Antergos wallpapers](https://github.com/Antergos/wallpapers)

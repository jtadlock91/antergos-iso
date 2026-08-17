---
title: Restoring Languages
layout: default
nav_order: 15
---

# Restoring non-English languages

This ISO ships **English-only** by default. Non-English UI translations for KDE and Qt are stripped via `NoExtract` in `pacman.conf` — about 608MB on a full install (`/usr/share/locale/` + `/usr/share/qt6/translations/`), the single largest space saving found in this fork, well ahead of any individual package removed.

This never breaks anything: every app's UI strings are compiled in as English by default, and a missing translation file just means it falls back to that — never a crash, never a missing feature, just no non-English UI text.

## Why this is safe to change per-install

The rule lives in `pacman.conf`, not baked into any package, so it's just a few lines in a text file you already have full control over on your own installed system.

## How to restore a language

`NoExtract` only affects *future* extraction — removing the rule doesn't retroactively restore files that were already skipped, so packages need to be re-extracted after the rule is gone.

**1. Edit `/etc/pacman.conf`** and remove (or comment out) these two lines:
```
NoExtract = usr/share/locale/* !usr/share/locale/en* !usr/share/locale/locale.alias
NoExtract = usr/share/qt6/translations/* !usr/share/qt6/translations/*en*
```

**2. Force re-extraction of the affected packages** — `--overwrite` makes pacman re-extract files even if the package version hasn't changed, which is necessary here since a normal reinstall of an already-current package does nothing:
```
sudo pacman -S --overwrite '*' plasma-desktop plasma-workspace kde-cli-tools qt6-base qt6-tools
```
Add any other specific packages tied to the language/app you're missing translations for.

**3. Reboot or log out and back in** for KDE to pick up the newly-restored translation files.

## If you'd rather not do this yourself

Open an issue on this repo, or reach out directly — this is a niche need most installs will never hit, and it's genuinely easier for the person who wrote the `NoExtract` rule to walk through it than to reverse-engineer from scratch.

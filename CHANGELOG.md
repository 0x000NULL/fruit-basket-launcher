# Changelog

## v0.2.0

The Basket and Downloads tabs: fruits can be installed and updated.

- Basket tab: In the basket, Ready to install and Still growing, with a
  status line on each card; FIND matches a fruit, console or extension;
  arrow keys move the selection and Z does the big button.
- The aside: Install, Update, Open (starts the emulator) or Try again, with
  the three step bars while it runs; What's new with Full notes; Open games/;
  the setup list; Stable/Nightly per fruit (switches to a kept build when it
  is on disk, else downloads it); sizes on disk; builds per platform with
  this PC marked; README, compatibility list and releases links; "Tell me
  when it's ripe" for growing fruits. Narrow windows show it as a sheet.
- Downloads tab: the running job with its steps, failure cards that say
  what happened on disk (signature, network, install) with Try again, the
  queue, and Earlier from `launcher/history.log`.
- Header: Update all · N queues every out-of-date fruit; the Downloads tab
  shows a count. Footer shows the running job's step and percent.
- "Install updates without asking" queues updates when a new feed arrives.
- Roll back and Uninstall are shown but not active yet.

## v0.1.0

The skeleton.

- Reads the signed feed from projects.ethanaldrich.net: minisign check against
  the built-in key, replayed older feeds refused, last good copy cached for
  offline use.
- Install pipeline (not yet in the UI): download with a size cap, SHA-256
  check against the feed, atomic extract and switch, Pomegranate's saves
  carried to the new build, keep-N pruning, uninstall.
- Window frame: Library, Basket, Downloads and Settings tabs, FIND, footer
  hints, launcher-update banner; regular, compact and narrow layouts; paper
  and night themes following the system.
- Settings tab, saved to `settings.toml` with unknown keys kept.

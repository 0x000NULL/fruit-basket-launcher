# Changelog

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

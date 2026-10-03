# Progress

The handoff document: where each milestone stands, what is waiting on
other repos, and the known gaps. Design: `docs/mocks/` (32 screens). Code
map: `docs/ARCHITECTURE.md`.

## Milestones

| | Milestone | State | Tag |
|---|---|---|---|
| M0 | Site: signed `feed.json`, `LAUNCHER` / `NIGHTLY` files, `make sign`, dump-list tool | done; deployed 2026-10-03; site main is at `26ceac2`, and the live feed is verified | — |
| M1 | Skeleton: window, frame, tabs, themes, three layouts, Settings | done | v0.1.0 |
| M2 | Basket tab, install pipeline in the UI, Downloads tab, history | done | v0.2.0 |
| M3 | Library: scan, covers / list, play, play time, compat, dump check | done | v0.3.0 |
| M4 | Lifecycle: rollback, uninstall, Move basket, keep-N in the UI, watch alerts, free space, `{data}` | done; the site's Pomegranate `{data}` feed is live | v0.4.0 |
| M5 | Controller: pad navigation, couch mode, saves view, Map buttons | done (v0.5.1: couch arguments only for listed builds) | v0.5.0 |
| M6 | Self-update, launcher builds hosted under `/fruit-basket/launcher/`, macos-x64 | done; the site mirrors each release (below) | v1.0.0 |

Each finished milestone gets an annotated tag. `Cargo.toml`'s version is
bumped in the same commit, and the release notes come from a
`## vX.Y.Z` section of CHANGELOG.md.

## Verified

- `cargo test`: 61 pass. Covers:
  - feed signature: a real signed feed passes; a tampered feed, the wrong key or an older feed is refused
  - install, switch, prune, uninstall; a bad archive leaves the current build running
  - a wrong hash deletes the download; a network failure changes nothing
  - queue order, dedupe and retry; history; library scan, titles, serials, saves, play time
  - compat matching; SHA-1 and CHD header hashes; the hash cache; launch sessions
  - rollback options and jobs for a build; the rollback, uninstall and move dialogs end to end in `App`
  - data migration (once, never overwriting, before pruning); `{data}` filled and an unfilled placeholder refused; a fruit's first Play moving its cards into data/ (with this test binary as the emulator)
  - the free-space check; the mover (destination rules, rename, copy check, a failure leaving the basket, never into itself); paths rebased after a move
  - the controller's focus (direction, Tab order, a vanished focus), and in `App`: the D-pad walking Settings, A ticking a box, a far control scrolled into view, Y into the aside and B back
  - save slots in both numberings with their pictures and notes; deleting one (and a pre-slots state with slot 0)
  - couch mode end to end: Start, LB/RB, the saves list, delete through the dialog, Load (the stand-in emulator gets `--slot`), no input while the game runs, B out
  - Map buttons: listen, bind with a swap, Done saves `[gamepad]`, Reset; the settings round trip keeps unknown keys
  - self-update: version order; staging refuses a wrong hash or a build that doesn't name itself, leaving nothing behind; the swap, the undo and the clean-up; an unwritable folder
  - `couch` arguments only for builds the feed lists
  - renders of every tab state, the dialogs, the ripe banner, the space failure, couch mode at 1280×720 and 1920×1080, the focus ring and the Map dialog
- `e2e_install_switch_and_refuse` (opt-in: `FRUITBASKET_E2E=1`; `FRUITBASKET_FEED` can point it at a local copy of the site instead) does all of this over HTTP. On 2026-10-03 it passed against the live feed:
  - installs Strawberry v1.4.0
  - fetches and matches its compat list
  - switches it to nightly with the stable build kept
  - refuses a job with a wrong hash, leaving nothing on disk
  - (v0.4.0, same day) rolls back to the kept stable build through the dialog and forward again, moves the basket, and uninstalls with the games kept
- A real window ran against the local site and wrote `settings.toml` on close.
- **Self-update, end to end on Windows (2026-10-03).** A debug v0.9.8 launcher used a local feed signed with a throwaway key, plus its own settings file and basket (`FRUITBASKET_CONFIG`). The feed offered v0.9.9.
  - It staged v0.9.9 in 2 s.
  - The next start swapped it in, exited 0, and started v0.9.9 with `--updated v0.9.8`.
  - The exe on disk then said v0.9.9, and the start after that removed `fruitbasket.old.exe` and the update folder.
  - A release build's `--version` reads through a pipe (`Command::output`), as the smoke test uses it.
  - Not done: clicking Restart now (the same spawn-and-quit path, untested by hand), macOS, Linux, and an unwritable folder by hand.
- The release workflow, run by hand on 2026-10-03, built and packaged all four targets, macos-x64 included.
- Not yet done by hand for M5: anything with a real controller. Untested by hand: the pad walking each tab, hot-plugging with "open in couch mode" on, couch mode's borderless window entering and leaving on each OS, and Map buttons with a real pad. CI builds the macOS (CoreGraphics) and Linux (xrandr) screen-size code but nothing runs it.
- Not yet done by hand: starting a real game from the Library (no ROM was available; `launch.rs` is tested with a stand-in program), clicking through the dialogs in a real window, and Move basket across two drives. The copy path is unit-tested, but every test move so far stayed on one volume and was a rename.

## Waiting on other repos

- **Strawberry `--slot N`**: done.
  - Released in v1.4.0 (2026-10-03) and mirrored to the site.
  - `strawberry/LAUNCHER` now has `load_slot {rom} --slot {slot}`, in site commit `0764b32`.
  - Strawberry's nightly is still the v1.3.1 build (`e8bc5e3`), which lacks the flag, so couch Load (M5) must offer Load only on builds that have it.
- **Pomegranate `--data` and `--slot`**: done.
  - Both flags shipped in v0.3.0 (2026-10-03, tag commit `f1a2c09`). It is on the site as stable v0.3.0, with nightly `f1a2c09` (the same zip).
  - The feed has used `--data` since `2026-10-03T17:36:29Z` (details below).
  - `ps2emu play <image> --data <DIR> [--slot N]`. Without a game, `ps2emu --data <DIR>` opens its library.
  - **Slots are 0–9** (`<stem>.s0.state` to `.s9.state`), not 1–8 like Strawberry's. The couch saves view (M5) needs a per-fruit slot range; take it from the slot files, or add a LAUNCHER key.
  - `--data` holds ps2emu.toml, cards/, states/, cache/ and screenshots. Migrate only ps2emu.toml, cards/ and states/; cache/ rebuilds itself. ps2emu migrates nothing. It exits 7 if DIR can't be created.
  - **The launcher side is done in v0.4.0:** `{data}` = `<root>/<fruit>/data/`, the `carry` files move there once, saves are read from it, and Open uses an `open` template.
  - **The site side is live** (site `ef573f7`, feed `2026-10-03T17:36:29Z`, checked here with minisign):
    - `launch play {rom} --data {data}`, `load_slot play {rom} --data {data} --slot {slot}`, `open --data {data}`, and `carry` unchanged.
    - Releases are only v0.3.0 (`oldest v0.3.0`), with stable v0.3.0 and nightly `f1a2c09`. Strawberry is unchanged apart from `open: []`.
  - Launchers before v0.4.0 can't start Pomegranate from this feed. With no self-update until M6, update them by hand from GitHub.
  - Builds before v0.3.0 have no `--data`, so they won't be offered (Ethan's call, 2026-10-03). Their zips stay on the site.
- **Couch flags** (asked of each emulator on 2026-10-03, with the same convention everywhere): `--fullscreen` for that run only, `--exit-on-quit` (the pause menu's Quit exits with 0), and a pad route to pause (Guide, or SELECT+START held 1 s).
  - **Pomegranate:** done in ps2emu v0.4.0 (tag commit `58d24fe`). It is live: the feed (generated 2026-10-03T18:14:23Z, site `26ceac2`) has stable v0.4.0, nightly `58d24fe`, `couch ["--fullscreen","--exit-on-quit"]`, and only v0.4.0 in releases (`oldest v0.4.0`). Launcher v0.5.1 gives `couch` only to builds the feed lists, so an installed or kept v0.3.0 doesn't get it.
  - **Strawberry:** asked, not started. It has no fullscreen and no pad route to its pause menu yet.
  - **Crabapple:** all four (`--slot N` 1–8, `--fullscreen`, `--exit-on-quit`, Guide or SELECT+START held 1 s) are on its main at `12c1737`, after v0.9.0 and not yet in a release. It isn't in the feed yet. When it goes in (Ethan's call): `launch {rom}`, `load_slot {rom} --slot {slot}`, `couch --fullscreen --exit-on-quit`, and `oldest` set to the first tag with the flags.
  - Without the flags a fruit still works from couch mode: in a window, back when the emulator exits.
- **Site, deployed at `26ceac2`** (2026-10-03, by Ethan through the site session):
  - `812813b`: Strawberry's nightly is now the v1.4.0 build (`5b00fe8`), which has `--slot`.
  - `5af8902`: the `couch` key, `site/fruit-basket/launcher/` for M6, and fetch-release checking a release's own `.sha256` files.
  - The launcher entry is still null. The site session was asked to mirror v0.5.1 (not v0.5.0), with Ethan's OK.
- **Dump lists**: none yet, because Ethan has no No-Intro or Redump DATs. The feed's `dumps` is null, so the Library says "No dump list for this fruit yet". With DATs: run `tools/make-dumps.py` in the site repo, then rebuild, re-sign and deploy.
- **Site**: deployed at `ef573f7`. The live `feed.json` and `.minisig` verify with key `9A7C56F99E6460E9`. nginx serves the feed as `application/json` and the signature as text, both no-cache.
  - The feed's LAUNCHER keys: `launch`, `load_slot`, `open` (all may use `{rom}`, `{slot}`, `{data}`), `carry`, and `oldest` (feedgen only; not in the feed).
  - The feed's top-level `launcher` entry is null: the site doesn't host launcher builds yet (M6).

## After v1.0.0

- **Mirror each release to the site** (the site session, with Ethan's OK): `./fetch-release.sh fruit-basket/launcher 0x000NULL/fruit-basket-launcher vX`, a notes line from this CHANGELOG, then deploy. v0.5.1 was asked for first; from v1.0.0 on, mirroring a release is what updates everyone.
- **The first real self-update** happens when v1.0.1 is mirrored and a v1.0.0 picks it up. The live path (site URL, 1-year nginx cache on release files, macOS and Linux) is only proven then.
- **Strawberry and Crabapple couch flags:** once a release has them, the site sets their `couch` key and raises `oldest` with it. No launcher change is needed.

## Known gaps

- A launcher update is checked only when the launcher opens (and with Settings → Check now). A launcher left open for days won't notice a new release.
- The macOS launcher is a bare binary in a tar.gz, not a signed `.app`. Gatekeeper may stop the first run of a download from the browser. Updates the launcher stages itself aren't affected.
- Couch mode's covers are placeholders, like the Library's. The saves list has no pictures, though Pomegranate writes a `.png` for each slot.
- The Map buttons dialog is taller than a 680-high window, and its bottom gets cut off there.
- Saves on a fruit without `load_slot` can be shown and deleted but not loaded. The dialog's button becomes Show file.
- The ripe banner shows only while the launcher is open; there is no OS notification.
- Roll back offers only what the feed lists: one nightly, plus the stable releases. Older nightlies can't be re-downloaded.
- Game covers are the mocks' striped placeholders. Real art (title captures, slot pictures) needs per-fruit probes.
- The library is rescanned at start and when the tab opens (if "Rescan when the launcher opens" is on). Nothing watches the folders.
- Library sort and view choices are not saved between runs.
- Tick boxes use basket-ui's filled square, not the mocks' check, to match the emulators.

## Deviations from the mocks

- Downloads → Earlier shows one row per job ("installed · stable v1.3.1"). The mock's separate "verified" and "installed" rows would say the same thing twice.
- A busy install shows "Verifying…" in the aside and the step bars, as in `Installing.png`; a queued one shows "Queued".
- `Couch-Saves.png` lists memory cards. The launcher lists save-state slots, because those are what an emulator can start from.
- Couch mode's Details shows the dump, saves and file in the hero; the mocks don't draw it.
- The ripe alert has no mock of its own; it uses the launcher-update banner from `Launcher-Update.png`.
- Move basket… asks to confirm in a dialog like the others. The mocks show only the button.

# Progress

The handoff document: where each milestone stands, what is waiting on
other repos, and the known gaps. Design: `docs/mocks/` (32 screens). Code
map: `docs/ARCHITECTURE.md`.

## Milestones

| | Milestone | State | Tag |
|---|---|---|---|
| M0 | Site: signed `feed.json`, `LAUNCHER` / `NIGHTLY` files, `make sign`, dump-list tool | done; site main is at `55e08e4` (2026-10-04), and the live feed is verified | — |
| M1 | Skeleton: window, frame, tabs, themes, three layouts, Settings | done | v0.1.0 |
| M2 | Basket tab, install pipeline in the UI, Downloads tab, history | done | v0.2.0 |
| M3 | Library: scan, covers / list, play, play time, compat, dump check | done | v0.3.0 |
| M4 | Lifecycle: rollback, uninstall, Move basket, keep-N in the UI, watch alerts, free space, `{data}` | done; the site's Pomegranate `{data}` feed is live | v0.4.0 |
| M5 | Controller: pad navigation, couch mode, saves view, Map buttons | done (v0.5.1: couch arguments only for listed builds) | v0.5.0 |
| M6 | Self-update, launcher builds hosted under `/fruit-basket/launcher/`, macos-x64 | done; v1.0.0 is live in the feed | v1.0.0 |

Each finished milestone gets an annotated tag. `Cargo.toml`'s version is
bumped in the same commit, and the release notes come from a
`## vX.Y.Z` section of CHANGELOG.md.

## Verified

- `cargo test`: 71 pass. Covers:
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
  - v1.1.0: pictures decode on a thread and shrink; `art` paths fill, and an unfilled one gives none; a save's picture beats `art`, which beats the stripes; Continue passes the newest slot, Start fresh doesn't, and a fruit without `load_slot` plays fresh; favorites filter, sort first in couch mode and follow a move; weekly buckets and totals from the session log, read back on a new run; shots of the pictures, favorites and Stats (three sizes, both themes)
  - v1.0.1: Map buttons in two columns at 1024×680 (Left/Right jump); sort and view round-trip through settings; Later holds for that build only; the folders' signature changes with a file added, grown or removed; Rescan finds a new file, an unchanged feed doesn't rescan and a newer one does
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

## The site and the other repos

As of 2026-10-04, the launcher waits on one thing: Strawberry and Crabapple writing save pictures and covers (below).

- **The site** is deployed at `55e08e4`.
  - The live feed was generated 2026-10-04T02:49:00Z. It verifies with key `9A7C56F99E6460E9`; this was checked here with minisign.
  - nginx serves the feed as `application/json` and the signature as text, both no-cache. Release files are cached for a year.
  - Feed keys per fruit, from its `LAUNCHER` file:
    - `launch`, `load_slot`, `open` and `couch` may use `{rom}`, `{slot}` and `{data}`.
    - `couch` goes after `launch` or `load_slot` in couch mode, never after `open`.
    - Also `carry`, and `oldest`, which feedgen uses and the feed doesn't carry.
  - **The launcher entry** is v1.1.0 for windows-x64, linux-x64, macos-arm64 and macos-x64.
    - The Windows zip's SHA-256 (`c4906c10…`) matches GitHub's `.sha256`, the feed and the live file.
    - v1.0.1 is on GitHub but was never mirrored; v1.1.0 includes it. v0.5.x was never mirrored either.
  - **`art`**, the new key: Pomegranate's is `{data}/cache/covers/{code}.png {data}/cache/covers/{stem}.png`. Older launchers ignore it.
- **Strawberry** is stable v1.5.0, with nightly `4ecce2e` (the same zip).
  - The feed lists only v1.5.0 (`oldest v1.5.0`). v1.4.0 and earlier stay on the site, out of the feed.
  - `launch {rom}`, `load_slot {rom} --slot {slot}`, `couch --fullscreen --exit-on-quit`.
  - Its slots are 1–8, beside the ROM. `--fullscreen` covers the primary display. Its pad route to pause is Guide, or Select+Start held 1 s.
  - Strawberry stays on fruit-basket v0.1.0, which is fine because v0.2.0 only adds things.
- **Pomegranate** (ps2emu) is stable v0.4.0, with nightly `58d24fe` (the same zip).
  - The feed lists only v0.4.0 (`oldest v0.4.0`).
  - `launch play {rom} --data {data}`, `load_slot play {rom} --data {data} --slot {slot}`, `open --data {data}`, `couch --fullscreen --exit-on-quit`.
  - `carry` is `ps2emu.toml cards states`; they move into `data/` once (cache/ rebuilds itself).
  - Its slots are 0–9 in `data/states/`, each with a `.png` and a `.toml` (`saved_at`). Deleting slot 0 also deletes a pre-slots `<stem>.state`. ps2emu asked that the launcher never write `ps2emu.toml`.
  - Launcher v0.5.1+ passes `couch` only to builds the feed lists, so an installed or kept v0.3.0 runs from couch mode in a window.
- **Crabapple** isn't in the feed. Its couch flags are on its main at `12c1737`, after v0.9.0 and not yet released: `--slot N` (1–8), `--fullscreen`, `--exit-on-quit`, and Guide or SELECT+START held 1 s.
  - Whether and when it goes in is Ethan's call: tag, then the site mirrors it.
  - Its LAUNCHER would then be `launch {rom}`, `load_slot {rom} --slot {slot}`, `couch --fullscreen --exit-on-quit`, with `oldest` set to that tag.
  - v0.9.0 ships windows-x64 and linux-x64 only.
- **Save pictures and covers for Strawberry and Crabapple** are in progress (asked 2026-10-04).
  - The Crabapple session is adding them once, to fruit-basket's `basket-app`, as an additive v0.3.0: `<stem>.sN.png` beside each state, and title captures as `<cache>/<app>/covers/<stem>.png`.
  - Both emulators then move to v0.3.0. Strawberry's session will report its commit.
  - Then the site's Strawberry LAUNCHER gets `art	{cache}/strawberry/covers/{stem}.png`, and Crabapple's gets the same with `crabapple` when it goes in the feed. No launcher change is needed.
- **fruit-basket** (the shared crates) is at v0.2.0 (`4416212`). It adds `Gamepads::connected()` and connect/disconnect events. No session owns that repo, and a change there should stay additive so the emulators can stay pinned where they are.
- **Dump lists:** none yet, because Ethan has no No-Intro or Redump DATs. The feed's `dumps` is null, so the Library says "No dump list for this fruit yet". With DATs: run `tools/make-dumps.py` in the site repo, then rebuild, re-sign and deploy.

## Releasing from here

- **Every launcher release the site mirrors updates every v1.0.0+ install.** Mirror one only when it's meant to ship:
  1. Tag it.
  2. With Ethan's OK, the site session runs `./fetch-release.sh fruit-basket/launcher 0x000NULL/fruit-basket-launcher vX`.
  3. It writes the notes line in the site's CHANGELOG.txt by hand, from this CHANGELOG: `fetch-release.sh` writes only "Release vX: builds in…", which would leave the update prompt with no notes.
  4. It deploys.
- **Check what ships first.** `gh workflow run release.yml` builds all four targets without releasing.
- **The first real self-update** is v1.1.0, mirrored 2026-10-04, when a v1.0.0 picks it up. Not yet seen happen. Only that proves the live path: the site URL, the year-long cache on release files, macOS and Linux.
- **A new couch-ready emulator release** needs only site work: set its `couch` key and raise `oldest` to the first build with the flags. No launcher change is needed.

## Known gaps

- The macOS launcher is a bare binary in a tar.gz, not a signed `.app`. Gatekeeper may stop the first run of a download from the browser. Updates the launcher stages itself aren't affected.
- Save pictures and covers come only from what an emulator writes. Pomegranate writes `.sN.png` slot pictures and `cache/covers/<serial>.png`; Strawberry and Crabapple keep their slot pictures inside the `.state` and their title captures under a path hash, so they show stripes until they write `<stem>.sN.png` and `<cache>/<app>/covers/<stem>.png` (asked of their sessions).
- Pomegranate's cover cache is keyed by the disc's boot serial. The launcher knows a serial only when it's in the file name, so other discs fall back to `{stem}`, which matches only when Pomegranate couldn't read a serial either.
- Sessions are logged from v1.1.0 on. Older play time counts in the totals but not in the weekly chart or the session lists.
- Saves on a fruit without `load_slot` can be shown and deleted but not loaded. The dialog's button becomes Show file.
- The ripe banner shows only while the launcher is open; there is no OS notification.
- Roll back offers only what the feed lists: one nightly, plus the stable releases at or after `oldest`. Older builds can't be re-downloaded (Strawberry v1.4.0 and Pomegranate v0.3.0 today); ones kept on disk still work.
- Tick boxes use basket-ui's filled square, not the mocks' check, to match the emulators.

## Deviations from the mocks

- Downloads → Earlier shows one row per job ("installed · stable v1.3.1"). The mock's separate "verified" and "installed" rows would say the same thing twice.
- A busy install shows "Verifying…" in the aside and the step bars, as in `Installing.png`; a queued one shows "Queued".
- `Couch-Saves.png` lists memory cards. The launcher lists save-state slots, because those are what an emulator can start from.
- Couch mode's Details shows the dump, saves and file in the hero; the mocks don't draw it.
- The ripe alert has no mock of its own; it uses the launcher-update banner from `Launcher-Update.png`.
- Move basket… asks to confirm in a dialog like the others. The mocks show only the button.

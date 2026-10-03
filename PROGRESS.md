# Progress

The handoff document: where each milestone stands, what is waiting on
other repos, and the known gaps. Design: `docs/mocks/` (32 screens). Code
map: `docs/ARCHITECTURE.md`.

## Milestones

| | Milestone | State | Tag |
|---|---|---|---|
| M0 | Site: signed `feed.json`, `LAUNCHER` / `NIGHTLY` files, `make sign`, dump-list tool | done; deployed 2026-10-03; site main is at `ef573f7`, and the live feed is verified | — |
| M1 | Skeleton: window, frame, tabs, themes, three layouts, Settings | done | v0.1.0 |
| M2 | Basket tab, install pipeline in the UI, Downloads tab, history | done | v0.2.0 |
| M3 | Library: scan, covers / list, play, play time, compat, dump check | done | v0.3.0 |
| M4 | Lifecycle: rollback, uninstall, Move basket, keep-N in the UI, watch alerts, free space, `{data}` | done; the site's Pomegranate `{data}` feed is live | v0.4.0 |
| M5 | Controller: pad navigation, couch mode, saves view, Map buttons | next | v0.5.0 |
| M6 | Self-update, launcher builds hosted under `/fruit-basket/launcher/` | | v1.0.0 |

Each finished milestone gets an annotated tag. `Cargo.toml`'s version is
bumped in the same commit, and the release notes come from a
`## vX.Y.Z` section of CHANGELOG.md.

## Verified

- `cargo test`: 47 pass. Covers:
  - feed signature: a real signed feed passes; a tampered feed, the wrong key or an older feed is refused
  - install, switch, prune, uninstall; a bad archive leaves the current build running
  - a wrong hash deletes the download; a network failure changes nothing
  - queue order, dedupe and retry; history; library scan, titles, serials, saves, play time
  - compat matching; SHA-1 and CHD header hashes; the hash cache; launch sessions
  - rollback options and jobs for a build; the rollback, uninstall and move dialogs end to end in `App`
  - data migration (once, never overwriting, before pruning); `{data}` filled and an unfilled placeholder refused; a fruit's first Play moving its cards into data/ (with this test binary as the emulator)
  - the free-space check; the mover (destination rules, rename, copy check, a failure leaving the basket, never into itself); paths rebased after a move
  - renders of every tab state, the dialogs, the ripe banner and the space failure
- `e2e_install_switch_and_refuse` (opt-in: `FRUITBASKET_E2E=1`; `FRUITBASKET_FEED` can point it at a local copy of the site instead) does all of this over HTTP. On 2026-10-03 it passed against the live feed:
  - installs Strawberry v1.4.0
  - fetches and matches its compat list
  - switches it to nightly with the stable build kept
  - refuses a job with a wrong hash, leaving nothing on disk
  - (v0.4.0, same day) rolls back to the kept stable build through the dialog and forward again, moves the basket, and uninstalls with the games kept
- A real window ran against the local site and wrote `settings.toml` on close.
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
- **Crabapple**: will need `--slot N` too once it ships.
- **Dump lists**: none yet, because Ethan has no No-Intro or Redump DATs. The feed's `dumps` is null, so the Library says "No dump list for this fruit yet". With DATs: run `tools/make-dumps.py` in the site repo, then rebuild, re-sign and deploy.
- **Site**: deployed at `ef573f7`. The live `feed.json` and `.minisig` verify with key `9A7C56F99E6460E9`. nginx serves the feed as `application/json` and the signature as text, both no-cache.
  - The feed's LAUNCHER keys: `launch`, `load_slot`, `open` (all may use `{rom}`, `{slot}`, `{data}`), `carry`, and `oldest` (feedgen only; not in the feed).
  - The feed's top-level `launcher` entry is null: the site doesn't host launcher builds yet (M6).

## For M5

- **Slot ranges differ:** Strawberry uses 1–8, Pomegranate 0–9. The saves view needs a per-fruit range: read it from the slot files, or add a LAUNCHER key.
- **Strawberry's nightly** (`e8bc5e3`) has no `--slot`, so on that build couch mode should offer Play only, not Load.
- **Where save states live:** `library::save_dirs` gives the folders, the data folder for Pomegranate. `basket_app::slots` reads the state headers.
- **Map buttons… and the setup list's Controller row** land with pad navigation.

## Known gaps

- Map buttons…, the couch button and the launcher-update banner's Restart now do nothing yet (M5–M6).
- The ripe banner shows only while the launcher is open; there is no OS notification.
- Roll back offers only what the feed lists: one nightly, plus the stable releases. Older nightlies can't be re-downloaded.
- Game covers are the mocks' striped placeholders. Real art (title captures, slot pictures) needs per-fruit probes.
- The library is rescanned at start and when the tab opens (if "Rescan when the launcher opens" is on). Nothing watches the folders.
- Library sort and view choices are not saved between runs.
- Tick boxes use basket-ui's filled square, not the mocks' check, to match the emulators.

## Deviations from the mocks

- Downloads → Earlier shows one row per job ("installed · stable v1.3.1"). The mock's separate "verified" and "installed" rows would say the same thing twice.
- A busy install shows "Verifying…" in the aside and the step bars, as in `Installing.png`; a queued one shows "Queued".
- The Controller row of the Basket's setup list is left out until Map buttons works (M5).
- The ripe alert has no mock of its own; it uses the launcher-update banner from `Launcher-Update.png`.
- Move basket… asks to confirm in a dialog like the others. The mocks show only the button.

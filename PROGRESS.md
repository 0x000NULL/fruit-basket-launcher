# Progress

The handoff document: where each milestone stands, what is waiting on
other repos, and the known gaps. Design: `docs/mocks/` (32 screens). Code
map: `docs/ARCHITECTURE.md`.

## Milestones

| | Milestone | State | Tag |
|---|---|---|---|
| M0 | Site: signed `feed.json`, `LAUNCHER` / `NIGHTLY` files, `make sign`, dump-list tool | done; deployed 2026-10-03 (site main at `850ed22`), live feed verified | — |
| M1 | Skeleton: window, frame, tabs, themes, three layouts, Settings | done | v0.1.0 |
| M2 | Basket tab, install pipeline in the UI, Downloads tab, history | done | v0.2.0 |
| M3 | Library: scan, covers / list, play, play time, compat, dump check | done | v0.3.0 |
| M4 | Lifecycle: rollback, uninstall, Move basket, keep-N in the UI, watch alerts, free space | next | v0.4.0 |
| M5 | Controller: pad navigation, couch mode, saves view, Map buttons | | v0.5.0 |
| M6 | Self-update, launcher builds hosted under `/fruit-basket/launcher/` | | v1.0.0 |

Each finished milestone gets an annotated tag. `Cargo.toml`'s version is
bumped in the same commit, and the release notes come from a
`## vX.Y.Z` section of CHANGELOG.md.

## Verified

- `cargo test`: 36 pass. Covers:
  - feed signature: a real signed feed passes; a tampered feed, the wrong key or an older feed is refused
  - install, switch, prune, uninstall; a bad archive leaves the current build running
  - a wrong hash deletes the download; a network failure changes nothing
  - queue order, dedupe and retry; history; library scan, titles, serials, saves, play time
  - compat matching; SHA-1 and CHD header hashes; the hash cache; launch sessions
  - renders of every tab state
- `e2e_install_switch_and_refuse` (opt-in: `FRUITBASKET_E2E=1`; `FRUITBASKET_FEED` can point it at a local copy of the site instead) does all of this over HTTP. On 2026-10-03 it passed against the live feed:
  - installs Strawberry v1.4.0
  - fetches and matches its compat list
  - switches it to nightly with the stable build kept
  - refuses a job with a wrong hash, leaving nothing on disk
- A real window ran against the local site and wrote `settings.toml` on close.
- Not yet done by hand: starting a real game from the Library. No ROM was available; `launch.rs` is tested with a stand-in program.

## Waiting on other repos

- **Strawberry `--slot N`**: done.
  - Released in v1.4.0 (2026-10-03) and mirrored to the site.
  - `strawberry/LAUNCHER` now has `load_slot {rom} --slot {slot}`, in site commit `0764b32`.
  - Strawberry's nightly is still the v1.3.1 build (`e8bc5e3`), which lacks the flag, so couch Load (M5) must offer Load only on builds that have it.
- **Pomegranate**: v0.3.0 (2026-10-03, tag commit `f1a2c09`) has both flags. The site session is mirroring it and adding `load_slot play {rom} --slot {slot}`.
  - `ps2emu play <image> --data <DIR> [--slot N]`. Without a game, `ps2emu --data <DIR>` opens its library.
  - **Slots are 0–9** (`<stem>.s0.state` to `.s9.state`), not 1–8 like Strawberry's. The couch saves view (M5) needs a per-fruit slot range; take it from the slot files, or add a LAUNCHER key.
  - `--data` holds ps2emu.toml, cards/, states/, cache/ and screenshots. Migrate only ps2emu.toml, cards/ and states/; cache/ rebuilds itself. ps2emu migrates nothing. It exits 7 if DIR can't be created.
  - **M4 work in this repo, as one change:**
    1. Fill `{data}` as `<root>/<fruit>/data/`.
    2. Move the current build's carry files there once.
    3. Change the site's `launch` to `play {rom} --data {data}` and `load_slot` to `play {rom} --data {data} --slot {slot}`.
    4. Drop `carry`.
    5. Point `library::saves` at the data folder.
  - Builds before v0.3.0 don't take `--data`, so a rollback to one has to fall back to the carry behaviour, or stop offering pre-v0.3.0 builds.
  - v0.2.0 and v0.1.0 releases are kept on the site.
- **Crabapple**: will need `--slot N` too once it ships.
- **Dump lists**: none yet, because Ethan has no No-Intro or Redump DATs. The feed's `dumps` is null, so the Library says "No dump list for this fruit yet". With DATs: run `tools/make-dumps.py` in the site repo, then rebuild, re-sign and deploy.
- **Site**: deployed. The live `feed.json` and `.minisig` verify with key `9A7C56F99E6460E9`. nginx serves the feed as `application/json` and the signature as text, both no-cache.

## Known gaps

- Storage in Settings shows space used, not free space (M4).
- Roll back… and Uninstall are drawn but inactive (M4). `Basket::switch`, `prune` and `uninstall` already work and are tested.
- Move basket…, Map buttons…, the couch button and the launcher-update banner do nothing yet (M4–M6).
- "Tell me when it's ripe" saves the watch list but raises no alert yet (M4).
- Game covers are the mocks' striped placeholders. Real art (title captures, slot pictures) needs per-fruit probes.
- The library is rescanned at start and when the tab opens (if "Rescan when the launcher opens" is on). Nothing watches the folders.
- Library sort and view choices are not saved between runs.
- Tick boxes use basket-ui's filled square, not the mocks' check, to match the emulators.

## Deviations from the mocks

- Downloads → Earlier shows one row per job ("installed · stable v1.3.1"). The mock's separate "verified" and "installed" rows would say the same thing twice.
- A busy install shows "Verifying…" in the aside and the step bars, as in `Installing.png`; a queued one shows "Queued".
- The Controller row of the Basket's setup list is left out until Map buttons works (M5).

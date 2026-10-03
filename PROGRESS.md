# Progress

The handoff document: where each milestone stands, what is waiting on
other repos, and the known gaps. Design: `docs/mocks/` (32 screens). Code
map: `docs/ARCHITECTURE.md`.

## Milestones

| | Milestone | State | Tag |
|---|---|---|---|
| M0 | Site: signed `feed.json`, `LAUNCHER` / `NIGHTLY` files, `make sign`, dump-list tool | done; site commits `8940a3b` and `0764b32`, not yet pushed or deployed | — |
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
- `e2e_install_switch_and_refuse` (opt-in: `FRUITBASKET_E2E=1` and `FRUITBASKET_FEED` pointing at a local copy of the site) does all of this over HTTP:
  - installs Strawberry v1.3.1 from the live site
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
- **Pomegranate**: needs `--slot N` for couch Load, and `--data DIR` so its saves can live outside the build folder. Not asked for yet: ps2emu was busy with other work. Until then the launcher moves the files on its `carry` list between builds.
- **Crabapple**: will need `--slot N` too once it ships.
- **Dump lists**: no fruit has a `dumps.txt` on the site yet. Run `tools/make-dumps.py` on the No-Intro (GBA) and Redump (PS2) DATs, then rebuild and re-sign the feed. Until then the Library says "No dump list for this fruit yet".
- **Site deploy**: the live site has no `feed.json` until the site is deployed (`make deploy`, or the same steps by hand; `make` isn't installed on this box).

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

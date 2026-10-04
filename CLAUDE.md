# CLAUDE.md: Fruit Basket launcher

One window for the Fruit Basket emulators: the game library, installing and
updating the fruits, a downloads queue, settings, couch mode for a controller,
and updating itself. All milestones (M0–M6) are done; v1.1.1 is live
(v1.1 added save pictures, covers, Continue, favorites and Stats).
It is Rust, built on the `fruit-basket` crates (`basket-ui`, `basket-app`,
`basket-build`, public repo `0x000NULL/fruit-basket`, pinned by git tag).

The docs:
- `PROGRESS.md`: the handoff (milestones, what waits on other repos, gaps)
- `docs/ARCHITECTURE.md`: the code map and the trust chain
- `docs/mocks/`: the design. Screens are in `screens/*.png`; where they conflict with the emulators' look, basket-ui tokens win.
- `README.md`: user-facing; `CHANGELOG.md`: per release

## Commands

    cargo test                       # unit tests + renders to target/shots/
    cargo run                        # the real window
    FRUITBASKET_FEED=http://127.0.0.1:8765/fruit-basket/feed.json cargo run
    FRUITBASKET_E2E=1 cargo test e2e -- --ignored --nocapture   # against the live site

For a local site, run `python -m http.server 8765 --bind 127.0.0.1` in
`../projects.ethanaldrich.net/site` after rebuilding and signing its feed.
The e2e test downloads real builds from the live site.

## Ground rules

- **Minimal.** No async runtime, no web view, no new dependency without a reason. Blocking I/O goes on a thread with an `mpsc` channel back to the UI.
- **The feed is the root of trust.** Anything downloaded is checked against a SHA-256 in the signed feed before it is used. Never add a path that skips that.
- **A failure never breaks the running build.** Extract to `.tmp`, rename, then rewrite `current` atomically (`basket::write_atomic`).
- **Immediate-mode UI.** Drawing code reads view structs and emits `Cmd`s; only `App::apply` changes state. New screens get a state in a `shots` test, compared by eye with the mock.
- **Dialogs** go through `App::modal` and `ui/modal.rs`. While one is open, the page draws with an empty input and `modal_keys` takes the keys.
- **Every control that can be pressed goes through `Ui::hot`**, never bare `ui.clicked`, so the controller can reach it. The grid items (covers, cards, tiles) are the exception, because the D-pad moves the selection there.
- **Couch mode never takes input while a game runs**, because the emulator reads the same pad.
- **Emulator arguments come only from feed templates** (`launch`, `load_slot`, `open`), filled by `launch::args`, which refuses an unfilled placeholder.
  - A fruit that uses `{data}` keeps its saves in `<fruit>/data/`.
  - `carry` files move there once (`Basket::migrate_data`), never overwriting.
- **Tests never touch real files.**
  - Each test uses its own temp basket.
  - `Settings::path()` is `None` under `cfg(test)`, so `save()` is a no-op. Keep it that way: a test once overwrote the real settings.toml.
- **The launcher replaces its own exe only through `update.rs`**: hashed against the signed feed, `--version` checked, the old exe kept until the new one has started. Never in tests (`cfg(test)`); the exe there is the test binary.
- **The private signing key never leaves `~/.minisign/`.** Only the public key is in `src/key.rs`.

## Git and releases

- **Public repo `0x000NULL/fruit-basket-launcher`.**
  - Commits and tags use the repo-local identity Ethan Aldrich <ethan@ethanaldrich.net>, not the global work address.
  - Check `git log --format='%ae %ce'` and the tagger before pushing.
- **No attribution trailers of any kind:** no Co-Authored-By, no Claude-Session, no "Generated with".
- **One tag per finished milestone** (M4 = v0.4.0 … M6 = v1.0.0); fixes are patch releases.
  - Bump `Cargo.toml`, and add a `## vX.Y.Z` section to CHANGELOG.md, in the release commit.
  - The release workflow takes its notes from that section.
- **GitHub-hosted runners only.** The repo is public, so it must never use the self-hosted BULV runners.
- **The site** (`../projects.ethanaldrich.net`, private) is committed separately and deployed by Ethan; don't push or deploy it unasked.
- **Other repos** (Strawberry, Crabapple, ps2emu) are changed by their own sessions; ask them rather than editing.
- **fruit-basket** (the shared crates) has no session of its own. Keep changes there additive, tag them, and pin the launcher to the tag.
- **A launcher release the site mirrors is an update for every v1.0.0+ install.** Tag freely, but the site mirrors one only with Ethan's OK.

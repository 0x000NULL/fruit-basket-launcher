# Fruit Basket launcher

One window for the Fruit Basket emulators: the game library across every
installed fruit, installing and updating the fruits themselves, a downloads
queue, and settings. Paper and night themes, three window sizes, and a couch
mode for controllers.

It is built on the same crates as the emulators
([fruit-basket](https://github.com/0x000NULL/fruit-basket): `basket-ui`,
`basket-app`, `basket-build`), so it looks and handles input exactly like them.
No async runtime, no web view: a `minifb` window drawn with tiny-skia.

Status: early. These tabs work:
- the Library: scan, play, play time, compat, dump check
- the Basket and Downloads: install and update
- Settings

Rollback, couch mode and self-update come next, a milestone at a time; see
[PROGRESS.md](PROGRESS.md) and [CHANGELOG.md](CHANGELOG.md).

## Using it

- **Basket.**
  - Every fruit from the site, in three sections: in the basket, ready to install, and still growing.
  - Pick one to install it, update it, or open it. Each fruit has its own Stable/Nightly choice.
  - **Update all** in the header queues every fruit with a newer build.
- **Downloads.**
  - The running job's Download, Verify and Install steps, then the queue, then the history.
  - A failure says what happened on disk (usually: nothing changed), with Try again.
- **Library.**
  - Everything the installed fruits can play, from each fruit's `games/` folder and any folders added with **Add folder…**.
  - **Play** or **Continue** starts the game in its fruit. The launcher keeps last played and play time.
  - Compat squares come from each fruit's compatibility list on the site. Where the site has a No-Intro or Redump list, each game is checked against it.
  - **Remove** hides a game and never deletes the file.
- **Keys.**
  - **Z**: the big button
  - **arrows**: move the selection
  - **/**: FIND
  - **PgUp/PgDn**: change tabs
  - **Esc**: quit

## Building

    cargo run                      # debug
    cargo build --profile dist     # what the releases ship
    cargo test                     # also renders every screen to target/shots/

Linux needs `libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev`.
Releases for Windows x64, Linux x64 and macOS arm64 are built from tags by
GitHub Actions.

## Where builds come from

The launcher reads one feed,
`https://projects.ethanaldrich.net/fruit-basket/feed.json`, with its
`feed.json.minisig` beside it. The trust chain:

1. **Signed feed.** The feed is signed with minisign, and the public key is compiled in (`src/key.rs`, key ID `9A7C56F99E6460E9`). A feed that does not verify is refused, and the last good copy is kept.
2. **No replays.** A feed older than the one already accepted is refused, so an old signed feed cannot be replayed.
3. **Checked downloads.** The feed carries the SHA-256 and size of every build, and of each fruit's compatibility and dump lists. A download that does not match is deleted before anything on disk changes.
4. **Atomic installs.** Each build is extracted beside the old one and renamed into place, then the fruit's `current` file is rewritten. A failure at any step leaves the running build alone.

Everything lives under `~/FruitBasket/` (movable in Settings):

    <fruit>/builds/<build>/   <fruit>/current   <fruit>/games/
    launcher/feed.json  launcher/feed.json.minisig  launcher/downloads/
    launcher/history.log  launcher/played.tsv  launcher/hashes.tsv  launcher/lists/

Settings are in `<config dir>/fruitbasket/settings.toml`.

### Testing against a local site

    FRUITBASKET_FEED=http://localhost:8765/fruit-basket/feed.json cargo run

Debug builds also accept `FRUITBASKET_KEY` (a minisign public key) so a test
feed can be signed with a throwaway key. Release builds ignore it.

## Design and code

- `docs/mocks/`: the 32 mock screens the UI follows, the interactive mock source, and the brand pack
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): modules, threads, the frame loop, and the release workflow

## License

MIT

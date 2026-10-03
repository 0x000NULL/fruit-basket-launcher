# Fruit Basket launcher

One window for the Fruit Basket emulators: the game library across every
installed fruit, installing and updating the fruits themselves, a downloads
queue, and settings. Paper and night themes, three window sizes, and a couch
mode for controllers.

It is built on the same crates as the emulators
([fruit-basket](https://github.com/0x000NULL/fruit-basket): `basket-ui`,
`basket-app`, `basket-build`), so it looks and handles input exactly like them.
No async runtime, no web view: a `minifb` window drawn with tiny-skia.

What it does:
- the Library: scan, play, play time, compat, dump check, saves
- the Basket and Downloads: install, update, roll back, uninstall
- Settings, including moving the basket and mapping the controller
- a controller everywhere, and couch mode
- updating itself from the site

See [PROGRESS.md](PROGRESS.md) for what's left and [CHANGELOG.md](CHANGELOG.md) for each release.

## Using it

- **Basket.**
  - Every fruit from the site, in three sections: in the basket, ready to install, and still growing.
  - Pick one to install it, update it, or open it. Each fruit has its own Stable/Nightly choice.
  - **Roll back…** goes to a build kept on disk or an older one from the site. **Uninstall** keeps games and saves unless asked.
  - **Tell me when it's ripe** on a growing fruit shows a banner once it is released.
  - **Update all** in the header queues every fruit with a newer build.
- **Downloads.**
  - The running job's Download, Verify and Install steps, then the queue, then the history.
  - A failure says what happened on disk (usually: nothing changed), with Try again.
- **Library.**
  - Everything the installed fruits can play, from each fruit's `games/` folder and any folders added with **Add folder…**.
  - **Play** or **Continue** starts the game in its fruit. The launcher keeps last played and play time.
  - Compat squares come from each fruit's compatibility list on the site. Where the site has a No-Intro or Redump list, each game is checked against it.
  - **Remove** hides a game and never deletes the file.
  - **Saves · N** lists the game's save states: **Load** starts the game from one, **Delete** removes it.
- **Couch mode.**
  - A full-screen view for a TV and a controller: **Start** or the header's controller button opens it, and **B** or **☰** goes back.
  - **LB/RB** switch system, **←/→** pick a game, **A** continues, **X** opens its saves, and **Y** shows details.
  - It opens by itself when a controller connects (Settings → Controller).
  - Fruits whose emulators support it start full screen, and go back to the launcher when you quit.
- **Keys and buttons.**
  - **Z** / **A**: the big button, or the control with the ring
  - **arrows** / **D-pad**: move the selection, or the ring
  - **C** / **Y**: into the aside; **X** / **B**: back
  - **Tab**, **Shift+Tab**: walk the controls
  - **/**: FIND
  - **PgUp/PgDn** / **LB/RB**: change tabs
  - **Enter** / **Start**: couch mode
  - **Esc**: quit (in couch mode: back to the desktop)
  - **Map buttons…** in Settings changes the controller's buttons.

## Building

    cargo run                      # debug
    cargo build --profile dist     # what the releases ship
    cargo test                     # also renders every screen to target/shots/

Linux needs `libasound2-dev libudev-dev libxkbcommon-dev libwayland-dev`.
Releases for Windows x64, Linux x64, macOS arm64 and macOS x64 are built
from tags by GitHub Actions, and mirrored to the site.

## Updates

The launcher checks the site's feed when it opens. If there is a newer
launcher, it downloads it in the background and checks it against the
signed feed. A banner then says it's ready: **Restart now**, or it goes in
the next time the launcher opens. If the launcher sits in a folder it
can't write to, the banner offers **Download** instead. Launchers before
v1.0.0 can't update themselves; get v1.0.0 by hand once.

## Where builds come from

The launcher reads one feed,
`https://projects.ethanaldrich.net/fruit-basket/feed.json`, with its
`feed.json.minisig` beside it. The trust chain:

1. **Signed feed.** The feed is signed with minisign, and the public key is compiled in (`src/key.rs`, key ID `9A7C56F99E6460E9`). A feed that does not verify is refused, and the last good copy is kept.
2. **No replays.** A feed older than the one already accepted is refused, so an old signed feed cannot be replayed.
3. **Checked downloads.** The feed carries the SHA-256 and size of every build, and of each fruit's compatibility and dump lists. A download that does not match is deleted before anything on disk changes.
4. **Atomic installs.** Each build is extracted beside the old one and renamed into place, then the fruit's `current` file is rewritten. A failure at any step leaves the running build alone.
5. **The launcher too.** Its own updates come from the same signed feed and are checked the same way. A new launcher must also report the right `--version` before it is staged.

Everything lives under `~/FruitBasket/` (movable in Settings):

    <fruit>/builds/<build>/   <fruit>/current   <fruit>/games/   <fruit>/data/
    launcher/feed.json  launcher/feed.json.minisig  launcher/downloads/
    launcher/history.log  launcher/played.tsv  launcher/hashes.tsv  launcher/lists/
    launcher/update/      a launcher update waiting for the next start

`<fruit>/data/` holds saves and settings for a fruit that takes a data
folder (Pomegranate). Its memory cards and states move there from the
build folder the first time it runs. Settings → Storage →
**Move basket…** moves all of it to another folder or drive.

Settings are in `<config dir>/fruitbasket/settings.toml`.

### Testing against a local site

    FRUITBASKET_FEED=http://localhost:8765/fruit-basket/feed.json cargo run

Debug builds also accept `FRUITBASKET_KEY` (a minisign public key) so a test
feed can be signed with a throwaway key, and `FRUITBASKET_CONFIG` (a
settings file) so a test never touches the real one. Release builds ignore
both.

## Design and code

- `docs/mocks/`: the 32 mock screens the UI follows, the interactive mock source, and the brand pack
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): modules, threads, the frame loop, and the release workflow

## License

MIT

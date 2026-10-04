# Architecture

The launcher is one Rust binary, `fruitbasket`. It draws with `basket-ui`
(tiny-skia + fontdue into a `minifb` window), reads input and gamepads and
keeps settings with `basket-app`, and talks to one site:
`projects.ethanaldrich.net/fruit-basket/`. It has no async runtime and no
web view: blocking I/O runs on plain threads, and each one hands its results
to the UI thread over an `mpsc` channel.

## The trust chain

```
feed.json + feed.json.minisig ──minisign, key in src/key.rs──▶ Feed
   │  refused if older than the last accepted feed (replay)
   ├─ assets[platform].sha256 ──▶ jobs.rs: download hashed as it streams;
   │                               mismatch = file deleted, nothing on disk changed
   ├─ compat / dumps {url, sha256} ──▶ lists.rs: cached list used only if it matches
   └─ launcher.assets[platform] ──▶ update.rs: hashed download, then `--version`
                                    must name the build before it is staged
```

Everything the launcher downloads is pinned by the signed feed. The signing
key lives offline on Ethan's machine (`~/.minisign/fruitbasket.key`). The
site signs with `make sign`, and `make deploy` refuses an unsigned or
stale feed. The public key and its ID `9A7C56F99E6460E9` are compiled in.
Debug builds also accept `FRUITBASKET_KEY` so tests can use a throwaway key,
and `FRUITBASKET_CONFIG` for a settings file other than the real one.

### Self-update

`App::check_launcher_update` runs once the feed is in (never under
`cfg(test)`). If `feed.launcher` is newer than `VERSION` and has a build
for this PC, `update::start` stages it on a thread:
1. Download it into `launcher/update/` and hash it against the feed.
2. Unpack it to `<build>.tmp`.
3. Run it with `--version`; it must print the build.
4. Rename it to `<build>/`, then write `update/staged` atomically.

The banner then offers Restart now, which saves settings, starts the
launcher again and quits.

At start, `main::swap_in_update` runs before the window opens. When
`staged` names a newer build:
1. Rename the running exe to `fruitbasket.old(.exe)`. A running exe can be
   renamed on every OS, but on Windows it can't be deleted.
2. Copy the staged exe to the old exe's path.
3. Start the new exe with `--updated <old version>`, and exit.

If any step fails, the old exe is put back. The next normal start deletes
the `.old` file and the update folder.

If the exe's folder can't be written (`update::can_replace`), nothing is
downloaded. The banner offers the asset's URL instead.

## Disk layout

```
~/FruitBasket/                    (Settings → Storage; Basket::default_root)
  <fruit>/builds/<build>/         one extracted build; .installed marks when it was installed
  <fruit>/current                 "<build>\t<channel>\n", rewritten atomically
  <fruit>/games/                  always scanned
  <fruit>/data/                   saves and settings, for fruits whose templates use {data}
  launcher/feed.json(.minisig)    last accepted feed, re-verified on load
  launcher/downloads/*.part       in-flight downloads
  launcher/history.log            Downloads → Earlier (history.rs)
  launcher/played.tsv             last played + play time per game (library.rs)
  launcher/hashes.tsv             SHA-1 per game, keyed by size + mtime (dumps.rs)
  launcher/lists/<fruit>.{compat,dumps}.txt
  launcher/update/                a launcher update: <build>/, and `staged` naming it (update.rs)
<config dir>/fruitbasket/settings.toml   (settings.rs, via basket_app::prefs)
```

### Saves: `carry` and `{data}`

An emulator that keeps saves and settings beside its exe names them in the
feed's `carry` list (Pomegranate: `ps2emu.toml cards states`). What happens
to them depends on the fruit's templates (`launch`, `load_slot`, `open`):

- **No `{data}`:** `Basket::switch` moves the `carry` files from the old
  build to the new one on every install, switch and rollback
  (`Fruit::build_carry`).
- **With `{data}`:** the fruit keeps them in `<fruit>/data/`, passed as
  `--data <dir>`. `Basket::migrate_data` moves the `carry` files there
  once, from the current build or else the newest that has them, and
  never overwrites (`Fruit::data_carry`). It runs after an install (before
  pruning, so no card is deleted with an old build), and before Play and
  Open, so an install from before the change moves over on first use.

`launch::args` fills `{rom}`, `{slot}` and `{data}`, and refuses a
placeholder it has nothing for, so a literal `{data}` never reaches an
emulator. The feed lists only a fruit's builds at or after `oldest` in its
LAUNCHER file. Today those are Pomegranate v0.4.0 and Strawberry v1.5.0,
the first builds with `--data` (Pomegranate) and the couch flags. Older
builds kept on disk can still be rolled back to.

A game started from couch mode gets the fruit's `couch` arguments after
its `launch` or `load_slot` ones (never after `open`), filled the same way:
e.g. `--fullscreen --exit-on-quit`, so the emulator fills the TV and its
pause menu's Quit comes back to the launcher. The site raises `oldest`
when it sets `couch`, and `Fruit::couch_args` gives the arguments only to a
build the feed lists, so an older build that is installed or kept for
rolling back never gets flags it doesn't know.

Save states are `<stem>.s<N>.state`, beside the game or in `save_dirs`.
`library::slots` reads the numbers from the files (Strawberry 1–8,
Pomegranate 0–9), the time from Pomegranate's `.sN.toml` `saved_at` or
else the file, and the files that go with each one, so `delete_slot`
removes them too (and a pre-slots `<stem>.state` with slot 0, which
Pomegranate would otherwise copy back).

## Modules

| File | What it does |
|---|---|
| `main.rs` | `--version`; swap in a staged update; `app::run()` |
| `folders.rs` | watching the game folders: a signature (path, size, mtime) on a thread every 15 s; a change rescans |
| `focus.rs` | the controller's focus: `Spot`s the frame drew, `next` (nearest in a direction, level ones first), `cycle` (Tab) |
| `app.rs` | `App`: state, the frame loop (poll → draw → apply `Cmd`s), `Ctx` (read-only view of the state shared by views and commands), the render and e2e tests |
| `key.rs` | embedded public key, key ID, feed URL (`FRUITBASKET_FEED` overrides) |
| `feed.rs` | feed types, `verify` / `fetch` / `load_cached` / `save_cached`, platform keys |
| `basket.rs` | the on-disk basket: install (extract to `.tmp`, flatten, rename, switch), switch, prune, uninstall, `migrate_data`, `exe`, `move_path` / `copy_tree` |
| `jobs.rs` | the download worker thread: a free-space check, then Download 0–70 %, Verify 70–85 %, Install 85–100 % |
| `queue.rs` | the UI side of the worker: one job at a time, no duplicates, failures kept until retried, `job_for`, `job_for_build`, `update_for`, `updates`, `rollback_options` |
| `history.rs` | `history.log` append and read |
| `library.rs` | game scan, titles, serials (GBA header code, disc serial in the name), save files and `save_dirs`, `slots` / `delete_slot`, `Played`, `rebase` (paths after a move) |
| `lists.rs` | fetches and caches compat and dump lists, checked against the feed |
| `compat.rs` | `compat.txt` parse; level by serial, then title |
| `dumps.rs` | `dumps.txt` parse, SHA-1 (CHD raw SHA-1 from the header), hash cache, hashing thread |
| `launch.rs` | template expansion (`{rom}`, `{slot}`, `{data}`; an unfilled one is an error), start, a thread that times the session |
| `mover.rs` | Move basket: where it goes, then rename or a checked copy on a thread |
| `update.rs` | the launcher's own update: `newer`, `offer` (not a build the player said Later to), stage (download, hash, unpack, `--version`), `apply_staged` / `undo` / `clean` at start |
| `shelf.rs` | the Library's state: games, lists, hashing, the running game (from a slot, with couch arguments), `view()`, `couch_rows()` |
| `settings.rs` | `Settings` (with `[gamepad]`, the launcher's controller map), unknown keys kept |
| `platform.rs` | OS dark mode, free space, screen size, open / reveal, `~` paths |
| `art.rs` | fruit icons (128 px, drawn at 64 and 32) and the basket mark |
| `window.rs` | the `minifb` window and input gathering; couch mode remakes it borderless at the screen's size |
| `ui/mod.rs` | `Tab`, `Size` (regular ≥ 1180, compact ≥ 820, narrow), `Cmd`, the `Ui` drawing context and shared controls |
| `ui/frame.rs` | header (tabs, FIND, Update all, couch button), the banner (launcher update, ripe fruit), footer |
| `ui/modal.rs` | the dialog: Roll back, Uninstall, Move basket, Saves, Delete save, Map buttons |
| `ui/couch.rs` | couch mode: laid out at the mocks' 1280×720 and scaled |
| `ui/library.rs`, `ui/basket.rs`, `ui/downloads.rs`, `ui/settings.rs` | the four tabs |

## The frame

`App::frame` runs about 80 times a second:

1. `poll()` drains the feed fetch, worker events (which advance the queue
   and write history), list downloads, hash results and finished game
   sessions.
2. `draw()` builds a view struct for the current tab, borrowing the state
   field by field. It draws the header, the tab and the footer. Drawing is
   immediate mode: a click or key becomes a `Cmd` and changes nothing yet.
3. `apply()` runs each `Cmd` after the frame: change state, save settings,
   queue a job, start a game.

While a dialog (`App::modal`) is open, the page under it is drawn with an
empty input, so nothing on it reacts. The dialog gets the real input, and
`modal_keys` replaces the page's shortcuts.

### The controller's focus

Every control that can be pressed goes through `Ui::hot(label, box)`: it
reports a click, or A while it holds the focus, and registers a `Spot`
(the label, `#1`, `#2`… when one repeats; its box; and its `Area`: the
scrolling list, the aside, or the fixed header and footer). The next frame's
D-pad steps through those spots with `focus::next`. A new control needs no
navigation code. `App::focus` holds the focused label: `None` on the
Library and Basket means the grid, where the D-pad moves the selection
instead and **Y** steps into the aside. When the focus moves, the page or
the aside scrolls to show it.

Couch mode (`App::couch`) replaces the whole frame with `ui::couch` and its
own keys (`couch_keys`). While a game runs it takes no input, because the
emulator is reading the same pad. `run` asks `Video::set_couch` to match
it after every frame. A controller plugging in (basket-app's
`PadPoll::connected`) opens it when Settings says so.

The view structs (`BasketView`, `LibraryView`, …) hold everything a tab
needs, so the tabs can be rendered headless. The `shots` tests in `app.rs`
render every state at 1280×900, 1024×680 and 640×880 in both themes to
`target/shots/`, to compare with `docs/mocks/screens/` by eye. Couch mode
renders at 1280×720 and 1920×1080.

## Threads

| Thread | Started by | Sends |
|---|---|---|
| feed fetch | `App::check_feed` | `Result<Fetched, FeedError>` |
| `downloads` | `Worker::start` (lives for the app) | `jobs::Event` |
| list fetch | `Shelf::lists` | `(fruit, Kind)` per list saved |
| `hashing` | `Shelf::start_hashing` | `((path, size, mtime), sha1)` |
| game session | `launch::start` | `Session` when the emulator exits |
| basket move | `mover::start` | `Progress` (percent, then done or the error) |
| launcher update | `update::start` | `Upd` (staged, or why not) |

## Releases

A pushed `v*` tag runs `.github/workflows/release.yml` on GitHub-hosted
runners. It runs the tests, then `cargo build --profile dist` for:
- windows-x64 (zip)
- linux-x64 and macos-arm64 (tar.gz)
- macos-x64 (tar.gz), cross-built on the Apple Silicon runner, with its
  tests left to macos-arm64

Each archive gets a `.sha256`. The GitHub release's notes are the tag's
`## vX.Y.Z` section of CHANGELOG.md. Run by hand (`gh workflow run
release.yml`), the workflow builds and packages every target without
releasing. The repo is public, so it must never use self-hosted runners.

The site mirrors each release with `./fetch-release.sh fruit-basket/launcher
0x000NULL/fruit-basket-launcher vX`, which checks the `.sha256` files. The
feed's `launcher` entry is then the newest mirrored release.

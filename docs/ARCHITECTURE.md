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
   └─ compat / dumps {url, sha256} ──▶ lists.rs: cached list used only if it matches
```

Everything the launcher downloads is pinned by the signed feed. The signing
key lives offline on Ethan's machine (`~/.minisign/fruitbasket.key`). The
site signs with `make sign`, and `make deploy` refuses an unsigned or
stale feed. The public key and its ID `9A7C56F99E6460E9` are compiled in.
Debug builds also accept `FRUITBASKET_KEY` so tests can use a throwaway key.

## Disk layout

```
~/FruitBasket/                    (Settings → Storage; Basket::default_root)
  <fruit>/builds/<build>/         one extracted build; .installed marks when it was installed
  <fruit>/current                 "<build>\t<channel>\n", rewritten atomically
  <fruit>/games/                  always scanned
  launcher/feed.json(.minisig)    last accepted feed, re-verified on load
  launcher/downloads/*.part       in-flight downloads
  launcher/history.log            Downloads → Earlier (history.rs)
  launcher/played.tsv             last played + play time per game (library.rs)
  launcher/hashes.tsv             SHA-1 per game, keyed by size + mtime (dumps.rs)
  launcher/lists/<fruit>.{compat,dumps}.txt
<config dir>/fruitbasket/settings.toml   (settings.rs, via basket_app::prefs)
```

Pomegranate keeps its settings, memory cards and states beside its exe. The
feed's `carry` list (`ps2emu.toml cards states`) names those files, and
`Basket::switch` moves them to each new build. A `--data DIR` flag in ps2emu
would make this unnecessary (see PROGRESS.md).

## Modules

| File | What it does |
|---|---|
| `main.rs` | module list; `app::run()` |
| `app.rs` | `App`: state, the frame loop (poll → draw → apply `Cmd`s), `Ctx` (read-only view of the state shared by views and commands), the render and e2e tests |
| `key.rs` | embedded public key, key ID, feed URL (`FRUITBASKET_FEED` overrides) |
| `feed.rs` | feed types, `verify` / `fetch` / `load_cached` / `save_cached`, platform keys |
| `basket.rs` | the on-disk basket: install (extract to `.tmp`, flatten, rename, switch), switch, prune, uninstall, `exe` |
| `jobs.rs` | the download worker thread: Download 0–70 %, Verify 70–85 %, Install 85–100 % |
| `queue.rs` | the UI side of the worker: one job at a time, no duplicates, failures kept until retried, `job_for`, `update_for`, `updates` |
| `history.rs` | `history.log` append and read |
| `library.rs` | game scan, titles, serials (GBA header code, disc serial in the name), save files, `Played` |
| `lists.rs` | fetches and caches compat and dump lists, checked against the feed |
| `compat.rs` | `compat.txt` parse; level by serial, then title |
| `dumps.rs` | `dumps.txt` parse, SHA-1 (CHD raw SHA-1 from the header), hash cache, hashing thread |
| `launch.rs` | launch template expansion, start, a thread that times the session |
| `shelf.rs` | the Library's state: games, lists, hashing, the running game, `view()` |
| `settings.rs` | `Settings`, unknown keys kept |
| `platform.rs` | OS dark mode, open / reveal, `~` paths |
| `art.rs` | fruit icons (128 px, drawn at 64 and 32) and the basket mark |
| `window.rs` | the `minifb` window and input gathering |
| `ui/mod.rs` | `Tab`, `Size` (regular ≥ 1180, compact ≥ 820, narrow), `Cmd`, the `Ui` drawing context and shared controls |
| `ui/frame.rs` | header (tabs, FIND, Update all, couch button), update banner, footer |
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

The view structs (`BasketView`, `LibraryView`, …) hold everything a tab
needs, so the tabs can be rendered headless. The `shots` tests in `app.rs`
render every state at 1280×900, 1024×680 and 640×880 in both themes to
`target/shots/`, to compare with `docs/mocks/screens/` by eye.

## Threads

| Thread | Started by | Sends |
|---|---|---|
| feed fetch | `App::check_feed` | `Result<Fetched, FeedError>` |
| `downloads` | `Worker::start` (lives for the app) | `jobs::Event` |
| list fetch | `Shelf::lists` | `(fruit, Kind)` per list saved |
| `hashing` | `Shelf::start_hashing` | `((path, size, mtime), sha1)` |
| game session | `launch::start` | `Session` when the emulator exits |

## Releases

A pushed `v*` tag runs `.github/workflows/release.yml` on GitHub-hosted
runners: test, `cargo build --profile dist` for windows-x64 (zip),
linux-x64 and macos-arm64 (tar.gz), a `.sha256` per archive, and a GitHub
release whose notes are the tag's `## vX.Y.Z` section of CHANGELOG.md. The
repo is public, so it must never use self-hosted runners.

# Changelog

## v1.1.2

- **Start fresh really starts fresh.** A fruit can give its own arguments for Start fresh with the feed's new `fresh` key (e.g. `{rom} --no-resume`). The Library's Start fresh link and couch mode's Start fresh row use them, with the couch arguments after them in couch mode. Play, and Continue for a game with no saves, still use the fruit's usual launch.
  - This is for Crabapple, which picks up where you left off unless asked not to.
  - Without the key, Start fresh starts the game as before.

## v1.1.1

- **A fruit can say which save slots it loads.** The feed's new `slots` key (lowest and highest, e.g. `1 8`) hides any other save from the saves lists, the Saves count, covers and Continue. Without it, every slot shows, as before.
  - This is for Crabapple. Its resume state on quit, `.s9`, would otherwise show as Slot 9. Being the newest save, Continue would then start Crabapple with `--slot 9`, which it refuses.

## v1.1.0

Pictures, Continue, favorites and play stats.

- **Save pictures.** Couch mode's saves list and the Saves dialog show each save's picture, for emulators that write one next to the save (Pomegranate does today). In couch mode, the big cover shows the picked save.
- **Real covers.** A game's cover is its newest save's picture. Without one, it is a cover picture the emulator keeps, from the feed's new `art` key. Otherwise it is the striped placeholder, as before.
- **Continue loads your newest save.** The Library's Continue button, the Continue cards, A on the grid, and couch mode's A all start from it, and the button says which slot. **Start fresh**, under the button, starts without it. A game with no saves, or a fruit that can't load them, just plays.
- **Favorites.** Mark a game with the star beside its cover (or A in couch mode's Details). A **Favorites** chip shows only those, covers wear a star, and couch mode lists favorites first. They follow the games when the basket moves.
- **Stats**, the Library's third view beside Covers and List:
  - play time, this week, sessions and games played
  - play time by week for the last 12 weeks
  - time per fruit, the most played games, and recent sessions
  - It follows the chips and FIND.
  - A game's panel shows its sessions too.
  - Sessions are logged from this version on. Play time from before still counts in the totals.
- The sort control's Recent is now **Last played**.

## v1.0.1

Fixes, and the first update a v1.0.0 launcher installs by itself.

- **Map buttons** fits a short window. When the eleven rows won't fit (a 680-high window), they go into two columns, and Left and Right jump between them.
- **The Library keeps its sort and view** (Recent or A–Z, Covers or List) between runs.
- **A launcher left open checks for updates every 4 hours**, and never while a game runs or a download is going.
  - The setting is now "Check for updates automatically".
  - **Later** on a launcher update holds for that build. A recheck doesn't bring the banner back; a newer build does.
  - A recheck that finds the same feed doesn't rescan the library.
- **Rescan** in the Library toolbar. The game folders are also checked every 15 seconds, so a ROM copied in shows up without a restart.

## v1.0.0

The launcher updates itself.

- **Self-update.** When the site has a newer launcher, it is downloaded in the background.
  - It is checked against the SHA-256 in the signed feed, unpacked, and asked for its version.
  - Only then does the banner say it is ready. **Restart now**, or the next start, swaps it in.
  - If anything fails, the launcher you have keeps running.
  - In a folder the launcher can't write to (such as Program Files), the banner offers **Download** instead.
- **macOS on Intel**: releases now include `macos-x64`.
- `fruitbasket --version` prints the version.
- **Updating from v0.5.1 or earlier:** download v1.0.0 by hand once, from the site or GitHub. Earlier launchers can't update themselves.

## v0.5.1

- The feed's `couch` arguments go only to a build the feed still lists. A Pomegranate v0.3.0 that is installed or kept for rolling back runs from couch mode in a window, because it doesn't know `--fullscreen` or `--exit-on-quit`.

## v0.5.0

Controller: the whole launcher from a pad, couch mode, saves, and Map buttons.

- **A controller reaches everything.**
  - On the Library and Basket, the D-pad moves the selection and A plays or opens.
  - **Y** goes into the aside, where the D-pad walks every button, tick box and link. A presses the one with the ring, and **B** comes back.
  - Downloads and Settings are walked the same way.
  - LB and RB change tabs, and **Start** opens couch mode.
  - On the keyboard, Tab and Shift+Tab walk the controls, Z presses, and C is Y.
- **Couch mode**, from Start, ☰ or the header's controller button.
  - It is a full-screen view for the TV: the system tabs (LB/RB), a big cover, Continue, Saves and Details, and a strip of covers.
  - B, Esc or ☰ goes back to the desktop.
  - It opens by itself when a controller connects, unless that is turned off in Settings.
  - While a game runs, the emulator has the pad and the launcher waits.
- **Saves.** **Saves · N** lists the game's save states, newest first. **Load** starts the game from one, and **Delete** removes it after asking.
  - Couch mode has the same list, with Start fresh at the end.
  - Strawberry's slots 1–8 and Pomegranate's 0–9 both work.
  - A deleted Pomegranate save takes its picture and notes with it.
- **Couch arguments.** A fruit's new feed key `couch` (e.g. `--fullscreen --exit-on-quit`) is added after its launch arguments when a game starts from couch mode.
- **Map buttons…** (Settings → Controller) remaps the launcher's own controller buttons.
  - It is saved in settings.toml as `[gamepad]`, and Reset brings back the defaults.
  - The keyboard always works, and each emulator keeps its own map.
- Settings and the Basket's setup list show the connected controller.
- Built on fruit-basket v0.2.0.

## v0.4.0

Lifecycle: roll back, uninstall, move the basket, and fruits that ripen.

- **Roll back…** in the Basket aside opens a dialog listing the builds
  kept on disk, then older builds from the feed (downloaded first). Saves
  and settings stay. The card then shows "update ready".
- **Uninstall** removes the program; games and saves stay unless "Also
  delete games and saves" is ticked.
- Dialogs take the keys: Z confirms, X or Esc cancels, the arrows pick,
  Space ticks.
- **Move basket…** in Settings moves everything to another folder (a
  rename, or a checked copy across drives), and the library's play times
  and hashes follow. A failed move changes nothing.
- Settings → Storage shows free space; each fruit's games size now
  includes its data folder.
- An install is refused before downloading if the drive has less than
  three times the download free ("not enough space").
- **Ripe alerts**: a watched fruit that is released shows a banner with
  Install and Later.
- **Data folders.** A fruit whose feed templates use `{data}` keeps its
  saves and settings in `<basket>/<fruit>/data/`. The files the feed names
  in `carry` move there once, on the first install, Play or Open after the
  change. The feed can give an `open` template for opening a fruit with no
  game. A template placeholder with nothing to fill it is an error, never
  passed to the emulator. Pomegranate switches to this once the site's
  feed does; update the launcher first, because v0.3.0 would pass
  `{data}` through as text.
- Sizes from 1 GB up read in GB.

## v0.3.0

The Library tab: every game the installed fruits read, and Play.

- Scans each installed fruit's `games/` folder and the extra folders from
  Settings (three folders deep); a file in an extra folder goes to the first
  installed fruit that reads its extension. Titles come from the file name
  with the region tags dropped.
- Chips per fruit, Recent / A–Z, Covers / List, FIND by game, console or
  fruit; Continue shows the last three games played.
- Play / Continue starts the game in its fruit's current build with the
  feed's launch template; the launcher times the session and keeps last
  played and play time in `launcher/played.tsv`. One game at a time.
- Compat squares from each fruit's compatibility list, matched by serial
  (GBA header code, disc serial in the file name, or the dump list's) then
  title. Lists are downloaded once and used only if they match the SHA-256
  in the signed feed.
- Dump check against a fruit's No-Intro / Redump list when the site has
  one: games are hashed in the background once (CHDs by their header's raw
  SHA-1) and cached in `launcher/hashes.tsv`.
- The aside: cover, compat, Play, Saves · N (save states named after the
  game), Show file, Remove (hides the game; the file stays), last played,
  play time, dump, file. Narrow windows show it as a sheet.
- Add folder… works here and in Settings; the compact header no longer
  lets FIND run into the tabs.

## v0.2.0

The Basket and Downloads tabs: fruits can be installed and updated.

- Basket tab: In the basket, Ready to install and Still growing, with a
  status line on each card; FIND matches a fruit, console or extension;
  arrow keys move the selection and Z does the big button.
- The aside: Install, Update, Open (starts the emulator) or Try again, with
  the three step bars while it runs; What's new with Full notes; Open games/;
  the setup list; Stable/Nightly per fruit (switches to a kept build when it
  is on disk, else downloads it); sizes on disk; builds per platform with
  this PC marked; README, compatibility list and releases links; "Tell me
  when it's ripe" for growing fruits. Narrow windows show it as a sheet.
- Downloads tab: the running job with its steps, failure cards that say
  what happened on disk (signature, network, install) with Try again, the
  queue, and Earlier from `launcher/history.log`.
- Header: Update all · N queues every out-of-date fruit; the Downloads tab
  shows a count. Footer shows the running job's step and percent.
- "Install updates without asking" queues updates when a new feed arrives.
- Roll back and Uninstall are shown but not active yet.

## v0.1.0

The skeleton.

- Reads the signed feed from projects.ethanaldrich.net: minisign check against
  the built-in key, replayed older feeds refused, last good copy cached for
  offline use.
- Install pipeline (not yet in the UI): download with a size cap, SHA-256
  check against the feed, atomic extract and switch, Pomegranate's saves
  carried to the new build, keep-N pruning, uninstall.
- Window frame: Library, Basket, Downloads and Settings tabs, FIND, footer
  hints, launcher-update banner; regular, compact and narrow layouts; paper
  and night themes following the system.
- Settings tab, saved to `settings.toml` with unknown keys kept.

# Fruit Basket Launcher — mockups

Launcher for the Fruit Basket emulators: game library + emulator manager
(tabs: Library, Basket, Downloads, Settings), controller/couch mode,
compact and narrow window layouts, paper and night themes. 32 screens.

## What's in here

- `screens/` — PNG of every screen at its design size.
- `standalone/` — the interactive screens, runnable locally.
  `Launcher.dc.html` is the one real component; every other screen
  imports it with different props. `Couch.dc.html` is couch mode.
  `support.js` is the Design-component runtime (bundles React).
- `source/project/` — the exact files on the canvas (`canvas.json` is the
  board layout; images there point at the canvas asset store as `/_blob/<id>`).
- `brand/` — the Fruit Basket brand pack the mockups use (marks, icons, boards).

## Run the interactive screens

    cd standalone
    python3 -m http.server 8000
    # open http://localhost:8000/

Fonts (Archivo, Source Serif 4, IBM Plex Mono) load from Google Fonts,
so you need a network connection.

## Placeholders

Anything in brackets is a placeholder for real data: [PS2 game 1], [BUILD],
[NEW BUILD], [SIZE], [DATE], [HH:MM], [KEY ID], [SITE HOST], [CONTROLLER],
[VERSION], [EXTRA FOLDER]. Build targets assumed: Windows x64 (this PC),
macOS arm64, Linux x64.

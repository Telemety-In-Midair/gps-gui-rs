# Completed

Items struck through in `TODO.md`, summarized. Newest first.

## 2026-09-06

- **Concise menus.** Every page lost the line under its heading and under
  each section title, and every line that restated a control: the node
  settings' "Node: ..." echoes (the value now sits in the section title,
  "Wake check: 5 min"), the mode description, the sleep and window notes on
  the Link section, the append note on Logging, the RSSI and layer
  explanations on Settings. Labels were cut to a few words ("Position from
  the node", "Defaults", "Color key"); ranges and reasons moved to hovers or
  to `docs/ui.md`. `heading!` and `section!` no longer take a hint.
- **ArcGIS maps.** `[map] tiles = "arcgis"` on the Settings page ("Map
  tiles") draws Esri's streets and outdoor styles from the static basemap
  tiles service and satellite imagery from World Imagery, under a built-in
  API key that `[map] arcgis_key` replaces. The map's layer button cycles
  the layers the provider has, satellite among them; region downloads and
  the offline zoom fallback follow the source on screen, converting map zooms
  to the tile levels a 512 px style is fetched at. Every source now shows its
  credit line in the map's bottom left corner.
- **Make top bar a dropdown.** A chevron tab hangs from the top right corner
  and folds the map's controls bar away; the bar's button row centers in the
  width left of it. The zoom buttons moved to a column in the bottom right
  corner above the status bar (in on top, out below), on every platform, and
  fold with the bar, as does the new color key under the bar's left end.
- **Status bar height based on the dBm range.** `[status_bar] rssi_top_dbm`
  (-20) and `rssi_bottom_dbm` (-120); a bar is full at the top and empty at
  the bottom, linear in dBm, which is the log scale of received power.
- **Connecting and scanning in a color, with dots.** `[ui] busy` (solarized
  blue) on every line describing something in progress, with `busy_dots`
  cycling after it. Elapsed counts always show seconds (`elapsed_text`).
- **Higher contrast menu items.** The menu rows draw in the theme's strong
  text with a matching stroke, glyph included.
- **Menu buttons larger.** A row fills the screen less the page margin and
  `type.menu.margin`, up to `type.menu.row.width`; rows are taller and the
  label bigger.
- **Default paths.** `app-settings.toml` (was `gps-config.toml`; an old file
  is renamed once at startup) and `RADIO.toml` both live beside the tile
  cache, which on a phone is the one writable place. Every path field has a
  Copy button; on Android that goes through the clipboard service over JNI,
  since egui's own clipboard is a no-op there.
- **Should the TOML have an assets/ equivalent?** No: the repo's root
  `app-settings.toml` is the generated default and a test holds it to that,
  the same arrangement as `gps-gui.look`, so nothing can drift.
- **Node track per node.** `PointSource::Board(BoardId)`; the transports
  report the address of the node a link came up to, and each node keeps its
  own track, so switching nodes ends a path instead of joining it.
- **Node list in the TOML.** A name a node reports for itself is written into
  `[ble.names]` under its address and saved, so a rename on the node renames
  it in the file.
- **Remote pulse timeout.** `[lora] pulse_secs` (10): a remote node's marker
  beats while it was heard within the window.
- **Listening mode.** Firmware `CFG_MODE 3`: GPS and receiver up, nothing
  transmitted, BLE up throughout, persisted. A fourth button on the mode row.
- **Device is a node.** Every page and message says node; "Beacon" survives
  only as the map marker's code name.
- **Settings are dropdowns.** `preset_pick`: a short list of presets and a
  custom entry that opens the editor, over most numbers on the Settings and
  Bluetooth pages. The text-size and bar-opacity sliders are the exceptions.
- **Phone name and location.** `[phone] name` labels this device everywhere;
  `[phone] location` (off by default) runs its receiver, which the Android
  source thread now powers only while asked. `[ble] location` (on) makes the
  connected node's fix the position, its marker folding into yours.
- **Transparency slider for the map bars.** `[map] bar_opacity`, shared by
  the controls bar, the status bar and the key.
- **Markers select on a single tap**, read off the tile widget's response so
  a tap on a button is never also a marker pick. **Color key** on the map.
- **Status page separated.** Your position (with its source) in one section,
  the node - link, mode, its own fix, telemetry - under its own name in
  another.
- **Picker without address names**; **node name last** on the Bluetooth page.
- **Idle to stored off by default** (firmware: `0x18 = 0` is off) with a
  Disable button; **the tracker's BLE on period** is its own setting
  (`0x1A`), no longer the wake check's advertising window.
- **Connect versus Connect to sleeping.** Checked: the same thing on desktop
  and on Android with "any node"; with a node pinned on Android, only the
  sleeping one scans continuously and can catch a window. Documented.
- **Less verbose.** Every hover and hint rewritten shorter.

- **Make extras menu. Reduce normal menu count.** The menu is two pages now.
  The main one holds Map, Status, Bluetooth and More; More holds Points,
  Logging, Settings and Radio, with a Back row after them. Both are drawn by
  `menu_list` from a list of `(Page, label, icon)` rows, so a row to another
  menu page is no different from a row to a destination. The X in the corner
  leaves the menu from either page. Beacon was renamed Bluetooth throughout
  (page, icon, text module, docs); its glyph was already the Bluetooth rune.
- **Unify the sizes / CSS-like type, class and id.** The look sheet now
  cascades: `type` a kind of thing, `class` a variant of it, `id` one element
  on one page, most specific first. Every `Key` declares its parent, `Look`
  stores what each key holds of its own, and `Look::get` walks the chain.
  Four identical path-field keys became one class. `inherit` is a value the
  sheet carries, and the adjuster gained a level picker plus an Inherit
  button. Old paths still load through a rename table. The plot's grid width
  and tick labels and the adjuster's outline moved into the sheet with it.

## 2026-09-05

- **Set font.** 0xProto (SIL OFL 1.1) embedded in the binary and installed at
  the front of both egui font families, egui's own kept behind it as fallbacks.
  `src/fonts.rs`, called from `MyApp::new` before the first frame.
- **Toml color theme control.** Both themes are solarized, built from one set
  of roles in `src/solarized.rs`. `[ui] theme` = `light` (default), `dark` or
  `system`. The theme now covers what the `[ui]` overrides never reached: the
  text-edit fill, the faint stripe, the selection, the links and the frames.
  The palette's green/red/orange became the defaults of `ok`/`error`/`pulse`.
  Contrast between the roles is asserted in tests.

~~Import gps / sensor data?~~

Persist beacon notify interval in firmware flash?

Set font?

Set interval for accelerometer, gps, BLE updates. 

Need BLE to be more robust?

User stories

Toggle button to continually attempt to wake.

~~Tracking mode and north up mode should use the accelerometer as well but at a lower hz make this a setting. Pointing the arrow on the maker like north up mode.~~
Compass now runs in every mode: full rate for heading-up, `compass.arrow_hz`
(default 4) for the marker arrow in north-up and tracking. `compass.marker_arrow`
turns the latter off.

Should be a force reconnect from scratch button.

More statuses in the app.

Toml color theme control. (partly done)
`[colors] outline` and a new `[ui]` table (`ok`, `error`, `pulse`) replaced the
hardcoded colors, and `[ui] background` / `button` / `text` override the theme
(empty = follow it). What is left: the text-edit fill, the faint stripe and the
selection color, and nothing checks that an overridden pair stays readable.

More color changes.

App needs to read the stats over usb from ESP as well.

Have receiver mode to get info.

Disconnect button or toggle (Must force disconnect).

Need to configure advertising window.

Add some memory for basic settings (Toml). Automatic if nothing present.
Default path for toml.

Remove some buttons on top bar? (Need to choose)

Red pulsing icons at the top should be only pulsing for a time if pressed when not valid. Otherwise greyed out.

Maybe make top bar a dropdown?

Text goes behind the page menu dropdown button.

~~Need to be able to handel multiple ESP's BLE at once. (Probably one at a time? Names? Select from a list?)~~
One at a time, picked from a scanned list, named in the app config (`[ble.names]`).

Change `gps-config.toml` name.

Beacon track is shared across boards, so the drawn path can span two of them after switching. Split it per board? (Points model change)

Optimize.

GPS BLE mesh? 1 central shares coords with others over BLE?

Edit dialog is too small.

Better color theme. Something visible in poor conditions.

Clean up the pages.

~~Show/hide path toggle instead of delete paths on map bar.~~
~~Setting for toggling central path as well.~~
The bar button is a session-only master switch over both paths (`show_paths`);
`[track] show_path` / `[ble] show_path` say which ones a shown map draws. The
line to the beacon and its distance stay. Discarding points moved to Settings.

better documentation of systems. Maybe mermaid block diagram?

Add name to status page.

List of BLE addresses/names in toml

Mode for BLE sleep while transmitting

Scanning for multiple, should be able to do this while connected?

When do radio settings get applied exactly?

Wio needs to work independent of GPS being active

Currently lose GPS coords of device when swapping BLE connection. Should be a deletion list.

Need to store paths (with time?).

Toml for time since last rx for remote beacon to be inactive for pulse.

Calculate flight time calc settings?

Reset to defaults for radio configs

Read settings from wio.

Board mapping should be based address even if direct from board.

Where is default pulled from firmware.

Only save edits?

~~Connecting for, not properly resetting.~~
The clock now restarts on every request (even a re-sent one) and when a live
link drops, so the count is this attempt's rather than the whole session's.

Add heartbeat on radio while no fix?

Get address from esp over BLE.

~~If no current point marker at last.~~
A remote node's marker (and its popup/center/track targets) falls back to the
last recorded track point when the live position is gone (board switch), so a
known tracker never draws as a bare path.

all settings (most) should be dropdowns.

~~"Connected. The board stays awake until you disconnect." Status message can be even when not true.~~
While connected but nothing has come off any characteristic for 3 notify
intervals (min 10 s), the line says "Connected, but nothing from the board for
X." instead, and is not painted in the all-well color.

~~Need to make a logging mode, CSV with all stats for graphing. (Logging page with graph?)~~
A Logging page (`src/logging.rs` + `src/app/ui/logging.rs`) records one CSV row
per report - phone fix, board fix, node position or ping, telemetry - with the
distance to you and to a configurable fixed reference filled in on the same row
as the RSSI. The graph plots any stat against time or against another stat, so
distance-vs-RSSI is a scatter. Export copies the CSV into the phone's Downloads
through MediaStore (`src/export.rs`, no dex shim needed).

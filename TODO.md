~~Import gps / sensor data?~~

Persist beacon notify interval in firmware flash?

~~Set font?~~
0xProto, embedded (`src/fonts.rs`), in front of egui's own for both families.

Set interval for accelerometer, gps, BLE updates. 

Need BLE to be more robust?

User stories

Toggle button to continually attempt to wake.

~~Tracking mode and north up mode should use the accelerometer as well but at a lower hz make this a setting. Pointing the arrow on the maker like north up mode.~~
Compass now runs in every mode: full rate for heading-up, `compass.arrow_hz`
(default 4) for the marker arrow in north-up and tracking. `compass.marker_arrow`
turns the latter off.

~~Should be a force reconnect from scratch button.~~
Connect stays live while connected and reads "Reconnect": every press now bumps
the request epoch, which ends the running session wherever it had got to and
starts over from a scan.

More statuses in the app.
A map status bar (`[status_bar]`, off by default) now covers the LoRa side: a
bar graph of the last 10 receptions colored per node, and one node's signal,
age, satellites (red/green on fix) and speed, cycling every `cycle_secs` when
several are heard. Still missing: anything about our own link or the board.

~~Toml color theme control.~~
`[ui] theme` picks solarized light (default), dark, or the system's, and the
theme now owns the text-edit fill, the faint stripe and the selection as well.
`background` / `button` / `text` still override it. Still unchecked: whether an
overridden pair stays readable - only the themes themselves are contrast-tested.

More color changes.

App needs to read the stats over usb from ESP as well.

Have receiver mode to get info.

~~Disconnect button or toggle (Must force disconnect).~~
Disconnect takes effect on the press: the worker drops the attempt mid-connect
rather than after it, the link state and everything the board reported are
cleared there and then, and the old session's remaining events are fenced out
by epoch instead of landing on the next board.

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

~~Add name to status page.~~
The BLE section says which board, by the same name every other page uses - the
one stored on the board when it has one.

List of BLE addresses/names in toml
(`[ble.names]` is that list; a name stored on the board, set from the Beacon
page, wins over it.)

~~Mode for BLE sleep while transmitting~~
The board has three modes now (stored / idle / tracking) and the Beacon page
picks between them. The BLE off period is tracking's knob specifically, so
"BLE asleep while it keeps beaconing" is what tracking plus a non-zero off
period already is.

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



Add heartbeat on radio while no fix?

Get address from esp over BLE.



all settings (most) should be dropdowns.





- BLE scan should always show time scanning seconds. Seconds should show even when minutes show.
- Make a transparency slider for this and the maps top bars backgrounds.

- Make beacons single tap not double.
- Add key for beacon color.

- On mobile: needs to get automatic place to save/load radio config. Also path copy button.

- Make extras menu. Reduce normal menu count

- Make status more separate, needs to be clear what is what. User vs node location.

- Get rid of temporary BLE names under scan for board.
- Board name should be last item in beacon settings.

- Idle -> stored automatically should be off by default.

- `advertising window` should not be for both tracking on period and advertising window, separate these.
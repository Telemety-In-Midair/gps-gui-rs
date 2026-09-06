# Completed

Items struck through in `TODO.md`, summarized. Newest first.

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

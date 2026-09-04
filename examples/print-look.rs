//! Print the look sheet the app ships with, documented, for seeding a file to
//! hand-edit:
//!
//! ```sh
//! cargo run --example print-look > gps-gui.look
//! ```
//!
//! The adjuster's Save writes the same sheet where there is none yet, so this
//! is only for starting from a file rather than from the app.

fn main() {
    print!("{}", gps_gui_rs::look::Look::default().to_sheet());
}

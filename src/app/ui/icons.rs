//! The app's icon set, named once.
//!
//! Every glyph is a white SVG in `assets/icons/`, tinted to the current text
//! color where it is drawn (see [`super::widgets::icon_button`]) so the set
//! follows the theme rather than carrying its own colors.
//!
//! One function per glyph, so no page has to spell out a path into `assets/`.
//! It has to be a function rather than a macro or a constant:
//! `egui::include_image!` embeds the file relative to the source file the
//! *invocation* is written in, so a macro would resolve its path against
//! whichever page called it, and the `ImageSource` it builds is not something
//! `const` can evaluate. A function body is written here, so the path is this
//! file's and every caller gets the same glyph.
//!
//! Adding a glyph means adding the file and one function here.

pub(super) fn beacon() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/beacon.svg")
}

pub(super) fn center() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/center.svg")
}

pub(super) fn check() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/check.svg")
}

pub(super) fn close() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/close.svg")
}

pub(super) fn edit() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/edit.svg")
}

pub(super) fn heading() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/heading.svg")
}

pub(super) fn log() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/log.svg")
}

pub(super) fn map() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/map.svg")
}

pub(super) fn menu() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/menu.svg")
}

pub(super) fn north() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/north.svg")
}

pub(super) fn path() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/path.svg")
}

pub(super) fn path_off() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/path-off.svg")
}

pub(super) fn points() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/points.svg")
}

pub(super) fn radio() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/radio.svg")
}

pub(super) fn settings() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/settings.svg")
}

pub(super) fn status() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/status.svg")
}

pub(super) fn topo() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/topo.svg")
}

pub(super) fn track() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/track.svg")
}

pub(super) fn zoom_in() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/zoom-in.svg")
}

pub(super) fn zoom_out() -> egui::ImageSource<'static> {
    egui::include_image!("../../../assets/icons/zoom-out.svg")
}

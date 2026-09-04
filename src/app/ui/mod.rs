//! The view layer: what each page looks like, kept apart from what the app
//! does.
//!
//! [`crate::app`] owns the state and the per-frame loop; everything here draws.
//! The split inside this module is by *kind of thing*, so a change has one
//! obvious home:
//!
//! - [`pages`] - one file per page, each a list of declarations: this section,
//!   that control, bound to this field.
//! - [`widgets`] - the vocabulary those declarations are written in, macros
//!   included.
//! - [`theme`] - every size and spacing in the app, as a named measure.
//! - [`text`] - the long-form prose and hover texts.
//! - [`icons`] - the icon set, named.
//! - [`menu`] - the menu page and the corner toggle that opens it.
//! - [`mapdraw`], [`plot`], [`statusbar`] - the hand-painted pictures, kept out
//!   of the pages that frame them.
//! - [`adjust`] - the adjuster: pick a thing on any page and move the measures
//!   behind it, live, then write them to the look sheet.
//!
//! The measures themselves are not here: they are the look sheet's
//! ([`crate::look`]), which [`theme`] reads for the pages.

pub(super) mod adjust;
pub(super) mod icons;
mod mapdraw;
mod menu;
mod pages;
mod plot;
mod statusbar;
mod text;
mod theme;
mod widgets;

/// Size every control off the body text, with a touch-target floor under the
/// lot. Applied to the style by [`crate::app::MyApp::apply_ui_style`], beside
/// the text sizes it is derived from. `publish` puts the look up for a frame,
/// which the app loop does before it draws anything.
pub(super) use theme::{apply_spacing, publish};

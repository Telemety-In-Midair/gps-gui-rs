//! The pages, one file each.
//!
//! Every file here is a page's definition: what is on it, in what order, bound
//! to what state. The shared vocabulary they are written in lives beside them
//! in [`super::widgets`] and [`super::theme`], the prose in [`super::text`],
//! and the two pieces of drawing big enough to be their own thing - the map
//! and the log graph - in [`super::mapdraw`] and [`super::plot`].

mod beacon;
mod logging;
mod manual;
mod map;
mod points;
mod radio;
mod settings;
mod status;

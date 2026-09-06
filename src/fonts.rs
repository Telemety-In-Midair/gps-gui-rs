//! The app's typeface: 0xProto, embedded in the binary.
//!
//! 0xProto (<https://github.com/0xType/0xProto>) is a monospaced face drawn
//! for reading code, which is close to what these pages are: coordinates,
//! signal figures, MAC addresses and timings, where a digit that lines up with
//! the digit above it is worth more than a proportional one that does not.
//! It is used for both families, so the tables on the Status and Points pages
//! read in the same face as everything around them.
//!
//! Embedded rather than asked of the system: Android ships no such face, and a
//! layout measured in text heights would shift under a font it did not choose.
//! egui's own fonts stay behind it as fallbacks, which is what still draws the
//! glyphs 0xProto has no outline for.

/// The regular face, the only weight the app draws with. egui picks no bold
/// face of its own - emphasis here is color and stroke width, not weight - so
/// shipping one would be bytes nothing reads.
const REGULAR: &[u8] = include_bytes!("../assets/fonts/0xProto-Regular.ttf");

/// What the face is called in [`egui::FontDefinitions`].
const NAME: &str = "0xProto";

/// Put 0xProto at the front of both font families, egui's own left behind it
/// as fallbacks. Called once, from [`crate::app::MyApp::new`], before anything
/// is drawn: the text styles and every measure derived from them are sized off
/// whatever face is in place.
pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        NAME.to_owned(),
        std::sync::Arc::new(egui::FontData::from_static(REGULAR)),
    );
    // In front of the defaults rather than instead of them: the fallback chain
    // is what still finds the arrows and the emoji.
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, NAME.to_owned());
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_face_is_embedded_and_is_a_truetype_file() {
        // `0x00010000` is the sfnt version of a TrueType outline file, which is
        // what egui's rasterizer wants; an OTF/CFF file would start "OTTO".
        assert!(REGULAR.len() > 1024);
        assert_eq!(&REGULAR[..4], &[0x00, 0x01, 0x00, 0x00]);
    }

    #[test]
    fn it_leads_both_families_without_dropping_the_fallbacks() {
        // The same construction `install` does, checked without a context: a
        // font can only be installed into a live egui context, and there is no
        // window in a test.
        let mut fonts = egui::FontDefinitions::default();
        let defaults = fonts.families.clone();
        fonts.font_data.insert(
            NAME.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(REGULAR)),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family.clone())
                .or_default()
                .insert(0, NAME.to_owned());
            let list = &fonts.families[&family];
            assert_eq!(list[0], NAME);
            assert_eq!(&list[1..], &defaults[&family][..]);
        }
    }
}

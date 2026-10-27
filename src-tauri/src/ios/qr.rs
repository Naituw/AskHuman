//! QR rendering of pairing codes for the settings window.

use qrcode::render::svg;
use qrcode::{EcLevel, QrCode};

/// Standalone SVG (dark modules on a white quiet zone) of `text`.
pub fn svg(text: &str) -> Result<String, String> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M)
        .map_err(|e| format!("cannot render the QR code: {e}"))?;
    Ok(code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build())
}

/// Block-character rendering for terminals with a dark background: the light modules (and the
/// quiet zone) are the filled glyphs, so the code scans with the usual polarity.
pub fn text(text: &str) -> Result<String, String> {
    use qrcode::render::unicode::Dense1x2;
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::L)
        .map_err(|e| format!("cannot render the QR code: {e}"))?;
    Ok(code
        .render::<Dense1x2>()
        .quiet_zone(true)
        .dark_color(Dense1x2::Light)
        .light_color(Dense1x2::Dark)
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ios::wire::PairCode;

    #[test]
    fn a_pairing_code_fits_into_a_qr_code() {
        let code = PairCode {
            v: 1,
            h: "abcdef.ala.us-east-1.emqxsl.com".into(),
            p: 8883,
            w: 8084,
            u: "user1234".into(),
            pw: "x".repeat(24),
            r: "https://1234567890-abcdefghij.ap-hongkong.tencentscf.com/askhuman".into(),
            m: "a".repeat(26),
            mn: "Tian's MacBook Pro".into(),
            t: "b".repeat(26),
            ts: "c".repeat(43),
            ek: "d".repeat(43),
            x: 1_800_000_300,
        }
        .encode();
        let svg = svg(&code).unwrap();
        assert!(svg.starts_with("<?xml") || svg.starts_with("<svg"));
        assert!(svg.contains("</svg>"));
        let block = text(&code).unwrap();
        assert!(block.lines().count() > 20 && block.contains('\u{2588}'));
    }
}

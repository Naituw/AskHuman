//! Keep GTK's unset font DPI sentinel out of WebKit's page zoom calculation.

use gtk::prelude::ObjectExt;

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn webkit_dpi_representable(value: f64) -> bool {
    // WebKit converts screen DPI * 1024 to a signed integer before deriving zoom.
    let xft_dpi = value * 1024.0;
    xft_dpi >= 1.0 && xft_dpi <= f64::from(i32::MAX)
}

fn replacement_dpi(current: f64, gtk_dpi: i32, font_scale: Option<&str>) -> Option<f64> {
    if positive(current) {
        return None;
    }
    let base = if gtk_dpi > 0 {
        f64::from(gtk_dpi) / 1024.0
    } else {
        96.0
    };
    let scale = font_scale
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| positive(*value))
        .unwrap_or(1.0);
    let scaled = base * scale;
    Some(if webkit_dpi_representable(scaled) {
        scaled
    } else {
        base
    })
}

pub(super) fn ensure_valid_resolution() -> std::io::Result<()> {
    let settings = gtk::Settings::default().ok_or_else(|| {
        std::io::Error::other("GTK settings unavailable before popup construction")
    })?;
    let screen = gtk::gdk::Screen::default()
        .ok_or_else(|| std::io::Error::other("GTK screen unavailable before popup construction"))?;
    let current = screen.resolution();
    let font_scale = std::env::var("GDK_DPI_SCALE").ok();
    let replacement = replacement_dpi(
        current,
        settings.property::<i32>("gtk-xft-dpi"),
        font_scale.as_deref(),
    );
    if let Some(dpi) = replacement {
        // File-source gtk-xft-dpi does not update GDK resolution on all backends.
        // Set only this process's screen, before WebKit derives page zoom from it.
        screen.set_resolution(dpi);
    }
    let actual = screen.resolution();
    if !positive(actual) || (replacement.is_some() && !webkit_dpi_representable(actual)) {
        return Err(std::io::Error::other(
            "GTK screen font resolution remains invalid before popup construction",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserve_existing_positive_resolution_without_rescaling() {
        for dpi in [72.0, 96.0, 144.0, 192.0] {
            assert_eq!(replacement_dpi(dpi, 98304, Some("2")), None);
        }
    }

    #[test]
    fn replace_unset_or_nonfinite_resolution_with_positive_gtk_font_dpi() {
        for invalid in [-1.0, 0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(replacement_dpi(invalid, 147456, None), Some(144.0));
        }
    }

    #[test]
    fn default_to_96_dpi_when_gtk_font_dpi_is_unset() {
        for gtk_dpi in [-1024, -1, 0] {
            assert_eq!(replacement_dpi(-1.0, gtk_dpi, None), Some(96.0));
        }
    }

    #[test]
    fn honor_positive_font_scale_and_reject_invalid_or_overflowing_values() {
        assert_eq!(replacement_dpi(-1.0, 147456, Some("1.5")), Some(216.0));
        for scale in ["0", "-1", "NaN", "inf", "invalid", "1e308"] {
            assert_eq!(replacement_dpi(-1.0, 98304, Some(scale)), Some(96.0));
        }
        assert_eq!(replacement_dpi(-1.0, 1, Some("5e-324")), Some(1.0 / 1024.0));
    }

    #[test]
    fn keep_replacements_within_webkit_integer_dpi_bounds() {
        for scale in ["1e-6", "1e10"] {
            assert_eq!(replacement_dpi(-1.0, 98304, Some(scale)), Some(96.0));
        }
        let minimum = 1.0 / 1024.0;
        let maximum = f64::from(i32::MAX) / 1024.0;
        assert!(webkit_dpi_representable(minimum));
        assert!(webkit_dpi_representable(maximum));
        assert!(!webkit_dpi_representable(minimum / 2.0));
        assert!(!webkit_dpi_representable(maximum * 2.0));
        assert_eq!(replacement_dpi(-1.0, 1, Some("1")), Some(minimum));
        assert_eq!(replacement_dpi(-1.0, i32::MAX, None), Some(maximum));
        // The fallback does not take ownership of an already-positive setting.
        assert_eq!(replacement_dpi(minimum / 2.0, 98304, None), None);
    }
}

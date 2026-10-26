//! Keep GTK's unset font DPI sentinel out of WebKit's page zoom calculation.

use gtk::prelude::ObjectExt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::Manager;

const WEBKIT_CSS_DPI: f64 = 96.0;

/// WebKitGTK derives its CSS page scale from the validated GDK screen resolution.
///
/// The value is captured while the GTK main thread is creating a webview. Geometry code
/// only reads the managed number; it must never call GDK from async paths.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PageScale(f64);

/// Tauri-managed numeric state captured on the GTK main thread.
pub(crate) struct PageScaleState {
    bits: AtomicU64,
    initialized: AtomicBool,
}

impl Default for PageScaleState {
    fn default() -> Self {
        Self {
            bits: AtomicU64::new(PageScale::default().get().to_bits()),
            initialized: AtomicBool::new(false),
        }
    }
}

impl PageScaleState {
    fn get(&self) -> PageScale {
        PageScale::new(f64::from_bits(self.bits.load(Ordering::Acquire))).unwrap_or_default()
    }

    fn initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }

    fn set(&self, scale: PageScale) {
        self.bits.store(scale.get().to_bits(), Ordering::Release);
        self.initialized.store(true, Ordering::Release);
    }
}

impl Default for PageScale {
    fn default() -> Self {
        Self(1.0)
    }
}

impl PageScale {
    pub(crate) fn new(value: f64) -> Option<Self> {
        page_scale_representable(value).then_some(Self(value))
    }

    pub(crate) fn from_dpi(dpi: f64) -> Option<Self> {
        webkit_dpi_representable(dpi).then_some(Self(dpi / WEBKIT_CSS_DPI))
    }

    pub(crate) fn get(self) -> f64 {
        self.0
    }
}

fn positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn webkit_dpi_representable(value: f64) -> bool {
    // WebKit converts screen DPI * 1024 to a signed integer before deriving zoom.
    let xft_dpi = value * 1024.0;
    xft_dpi >= 1.0 && xft_dpi <= f64::from(i32::MAX)
}

fn page_scale_representable(value: f64) -> bool {
    positive(value)
        && value <= f64::from(i32::MAX) / WEBKIT_CSS_DPI
        && webkit_dpi_representable(value * WEBKIT_CSS_DPI)
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

pub(super) fn ensure_valid_resolution() -> std::io::Result<PageScale> {
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
    PageScale::from_dpi(actual).ok_or_else(|| {
        std::io::Error::other("GTK screen resolution cannot produce a valid WebKit page scale")
    })
}

pub(crate) fn page_scale<R: tauri::Runtime, M: Manager<R>>(manager: &M) -> PageScale {
    manager
        .try_state::<PageScaleState>()
        .map(|state| state.get())
        .unwrap_or_default()
}

pub(crate) fn initialize_page_scale<R: tauri::Runtime, M: Manager<R>>(
    manager: &M,
) -> std::io::Result<PageScale> {
    let state = manager.state::<PageScaleState>();
    if state.initialized() {
        return Ok(state.get());
    }
    let scale = ensure_valid_resolution()?;
    state.set(scale);
    Ok(scale)
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

    #[test]
    fn page_scale_uses_validated_dpi_and_rejects_invalid_values() {
        for (dpi, expected) in [(120.0, 1.25), (144.0, 1.5), (72.0, 0.75), (96.0, 1.0)] {
            assert_eq!(PageScale::from_dpi(dpi).unwrap().get(), expected);
        }
        for invalid in [
            0.0,
            -1.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::from(i32::MAX),
            f64::MAX,
        ] {
            assert!(PageScale::from_dpi(invalid).is_none());
            assert!(PageScale::new(invalid).is_none());
        }
        assert_eq!(PageScale::default().get(), 1.0);
    }

    #[test]
    fn managed_state_falls_back_to_one_and_stores_a_validated_scale() {
        let state = PageScaleState::default();
        assert!(!state.initialized());
        assert_eq!(state.get().get(), 1.0);
        state.set(PageScale::new(1.5).unwrap());
        assert!(state.initialized());
        assert_eq!(state.get().get(), 1.5);
    }
}

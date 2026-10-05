//! Pure geometry for a fixed right-side panel, with temporary horizontal split as a last resort.
use serde::Serialize;
pub const DIVIDER: f64 = 6.0;
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Side {
    #[default]
    Closed,
    Right,
    Inside,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Rect {
    pub fn right(self) -> f64 {
        self.x + self.width
    }
    pub fn bottom(self) -> f64 {
        self.y + self.height
    }
    pub fn intersection(self, other: Self) -> f64 {
        (self.right().min(other.right()) - self.x.max(other.x)).max(0.0)
            * (self.bottom().min(other.bottom()) - self.y.max(other.y)).max(0.0)
    }
}
// Preserve the original main width before reducing the preview, then shift the entire frame left.
// Only a work area narrower than main + readable preview requires a temporary split.
pub fn place_right(main: Rect, work: Rect, scale: f64, preview_width: f64) -> (Side, Rect) {
    if !scale.is_finite() || scale <= 0.0 || !work.width.is_finite() || work.width <= 0.0 {
        return (Side::Inside, main);
    }
    let available = work.width - main.width;
    let (side, width) = if available >= (320.0 + DIVIDER) * scale {
        (
            Side::Right,
            main.width + available.min((preview_width.max(320.0) + DIVIDER) * scale),
        )
    } else {
        (Side::Inside, work.width)
    };
    (
        side,
        Rect {
            x: main.x.clamp(work.x, work.right() - width),
            width,
            ..main
        },
    )
}
pub fn inside_main_width(total: f64, wanted: f64, fraction: Option<f64>) -> f64 {
    let usable = (total - DIVIDER).max(1.0);
    let minimum_preview = 320.0_f64.min(usable / 2.0);
    let minimum_main = 240.0_f64.min(usable / 2.0);
    fraction
        .map_or(wanted, |share| usable * share)
        .clamp(minimum_main, usable - minimum_preview)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shifts_left_to_keep_a_full_right_preview() {
        let main = Rect {
            x: 700.0,
            y: 80.0,
            width: 560.0,
            height: 652.0,
        };
        let work = Rect {
            x: 0.0,
            y: 25.0,
            width: 1440.0,
            height: 850.0,
        };
        let (side, frame) = place_right(main, work, 1.0, 700.0);
        assert_eq!(side, Side::Right);
        assert_eq!(frame.x, 174.0);
        assert_eq!(frame.width, 1266.0);
        assert_eq!(frame.y, main.y);
    }
    #[test]
    fn reduces_preview_before_narrowing_the_main_region() {
        let main = Rect {
            x: 100.0,
            y: 80.0,
            width: 560.0,
            height: 652.0,
        };
        let work = Rect {
            x: 0.0,
            y: 25.0,
            width: 1000.0,
            height: 850.0,
        };
        assert_eq!(
            place_right(main, work, 1.0, 900.0),
            (
                Side::Right,
                Rect {
                    x: 0.0,
                    width: 1000.0,
                    ..main
                }
            )
        );
        let (side, frame) = place_right(
            main,
            Rect {
                width: 800.0,
                ..work
            },
            1.0,
            900.0,
        );
        assert_eq!(side, Side::Inside);
        assert_eq!(frame.width, 800.0);
        assert_eq!(inside_main_width(800.0, 560.0, None), 474.0);
        assert_eq!(inside_main_width(1000.0, 560.0, None), 560.0);
    }
    #[test]
    fn handles_negative_coordinates_and_physical_scale_without_moving_vertically() {
        let main = Rect {
            x: -1200.0,
            y: 80.0,
            width: 1120.0,
            height: 1304.0,
        };
        let work = Rect {
            x: -2880.0,
            y: 50.0,
            width: 2880.0,
            height: 1700.0,
        };
        let (side, frame) = place_right(main, work, 2.0, 700.0);
        assert_eq!(side, Side::Right);
        assert_eq!(frame.x, -2532.0);
        assert_eq!(frame.width, 2532.0);
        assert_eq!(frame.y, 80.0);
    }
}

//! The pointer's way, as a hand moves it.

use eframe::egui::{self, Pos2};

/// Where a hand's pointer is a share `k` of its way from `from` to `to`:
/// on a bow to one side (its side and depth from the way itself, so the
/// same way bows the same), most of the way in the first half, past the
/// target by a few points near the end and back onto it.
pub fn hand_path(from: Pos2, to: Pos2, k: f32) -> Pos2 {
    let d = to - from;
    let len = d.length();
    if len < 1.0 {
        return to;
    }
    let n = egui::vec2(-d.y, d.x) / len;
    // A number in -1..1 from the way, the same each time.
    let seed = ((from.x * 12.9898 + from.y * 78.233 + to.x * 37.719 + to.y * 4.581).sin()
        * 43_758.547)
        .fract();
    let bow = (0.08 + 0.1 * seed.abs()) * len.min(400.0) * seed.signum();
    // Fast, then slowing down: an ease-out with a soft start.
    let e = 1.0 - (1.0 - k).powi(3);
    let s = e * e * (1.5 - 0.5 * e);
    let along = from + d * s;
    // The bow is deepest in the middle and gone at the ends.
    let side = n * bow * (4.0 * s * (1.0 - s));
    // Past the target near the end, and back.
    let past = (len * 0.025).min(6.0);
    let over = if k > 0.7 {
        let q = (k - 0.7) / 0.3;
        d / len * past * (std::f32::consts::PI * q).sin()
    } else {
        egui::Vec2::ZERO
    };
    along + side + over
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::pos2;

    /// A hand's way starts and ends where it should, bows off the straight
    /// line on the way, passes the target a little and is the same each
    /// time.
    #[test]
    fn the_pointer_goes_as_a_hand() {
        let (from, to) = (pos2(100.0, 500.0), pos2(700.0, 200.0));
        assert_eq!(hand_path(from, to, 0.0), from);
        assert!(hand_path(from, to, 1.0).distance(to) < 1e-3);
        let line = |p: Pos2| {
            let d = (to - from).normalized();
            ((p - from).x * d.y - (p - from).y * d.x).abs()
        };
        assert!(line(hand_path(from, to, 0.4)) > 10.0);
        // Past the target, along the way.
        let near = hand_path(from, to, 0.85);
        assert!((near - from).length() > (to - from).length());
        assert_eq!(hand_path(from, to, 0.3), hand_path(from, to, 0.3));
        // Most of the way in the first half.
        assert!(hand_path(from, to, 0.5).distance(to) < 0.25 * from.distance(to));
    }
}

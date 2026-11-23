//! A curve through points that the user places: a velocity curve, the
//! response of a controller, a transfer function.
//!
//! A [`Curve`] maps a value from 0.0 to 1.0 to another one from 0.0 to 1.0.
//! It runs through its points from the first to the last by x. Before the
//! first point it holds the value of that point, after the last one too.
//! Between the points it is straight or smooth, as its [`CurveShape`] says.
//!
//! The smooth shape is a monotone cubic (Fritsch-Carlson): it is smooth at
//! every point but never rises or falls where the points do not. Between
//! two points it stays between their values, so it never leaves 0.0 to 1.0.
//! Where the points rise, a harder hit never comes out softer than a softer
//! one. A natural cubic spline would overshoot between points that lie
//! close together.
//!
//! A curve is checked when it is created, also from JSON: a saved state or
//! a web UI can hold anything. Reading it allocates nothing and takes no
//! lock, so the audio thread can read it through a
//! [`State`](crate::State).
//!
//! ```ignore
//! #[beamer::model]
//! struct VelocityModel {
//!     #[state(id = "velocity_curve")]
//!     velocity_curve: State<Curve>,
//! }
//!
//! // In prepare():
//! VelocityProcessor { velocity_curve: self.velocity_curve.reader(), model: self }
//!
//! // In process_midi(), once per block:
//! let curve = self.velocity_curve.latest();
//! // For each note-on:
//! let velocity = curve.value_at(note_on.velocity as f64);
//! ```
//!
//! # JSON
//!
//! A curve is saved and reaches a web UI as its shape and its points:
//!
//! ```json
//! {"shape":"smooth","points":[{"x":0.0,"y":0.0},{"x":0.5,"y":0.7},{"x":1.0,"y":1.0}]}
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

/// A point of a [`Curve`], both coordinates from 0.0 to 1.0.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    /// The value that goes in.
    pub x: f64,
    /// The value that comes out.
    pub y: f64,
}

impl CurvePoint {
    /// A point at `x` and `y`.
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// How a [`Curve`] runs between its points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CurveShape {
    /// Straight lines from point to point.
    Linear,
    /// Smooth through every point without overshooting them: a monotone
    /// cubic.
    #[default]
    Smooth,
}

/// Why points do not make a [`Curve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveError {
    /// A curve needs at least two points.
    TooFewPoints,
    /// A coordinate is not a number from 0.0 to 1.0. Holds the index of
    /// the point.
    OutOfRange(usize),
    /// A point does not lie to the right of the one before it. Holds the
    /// index of the point.
    NotIncreasing(usize),
    /// A point lies closer than [`Curve::MIN_POINT_DISTANCE`] to the one before
    /// it by x. Holds the index of the point.
    TooClose(usize),
}

impl fmt::Display for CurveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFewPoints => write!(f, "a curve needs at least two points"),
            Self::OutOfRange(index) => {
                write!(f, "point {index} of the curve lies outside 0.0 to 1.0")
            }
            Self::NotIncreasing(index) => write!(
                f,
                "point {index} of the curve does not lie to the right of the point before it"
            ),
            Self::TooClose(index) => write!(
                f,
                "point {index} of the curve lies closer than {} to the point before it",
                Curve::MIN_POINT_DISTANCE
            ),
        }
    }
}

impl std::error::Error for CurveError {}

/// A curve through points, from 0.0 to 1.0 in and out.
///
/// See the [module documentation](self).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CurveData", into = "CurveData")]
pub struct Curve {
    shape: CurveShape,
    points: Vec<CurvePoint>,
    /// The slope of the curve at each point, for the smooth shape. Empty
    /// for the linear shape.
    slopes: Vec<f64>,
}

impl Curve {
    /// The closest two points may lie by x. Closer points would make the
    /// slopes between them overflow.
    pub const MIN_POINT_DISTANCE: f64 = 1e-9;

    /// A curve of `shape` through `points`.
    ///
    /// The points need to be sorted by x with no two closer than
    /// [`Curve::MIN_POINT_DISTANCE`]. Both coordinates of each need to be from 0.0
    /// to 1.0.
    pub fn new(shape: CurveShape, points: Vec<CurvePoint>) -> Result<Self, CurveError> {
        if points.len() < 2 {
            return Err(CurveError::TooFewPoints);
        }
        for (index, point) in points.iter().enumerate() {
            let in_range = |value: f64| (0.0..=1.0).contains(&value);
            if !in_range(point.x) || !in_range(point.y) {
                return Err(CurveError::OutOfRange(index));
            }
            if index > 0 {
                let distance = point.x - points[index - 1].x;
                if distance <= 0.0 {
                    return Err(CurveError::NotIncreasing(index));
                }
                if distance < Self::MIN_POINT_DISTANCE {
                    return Err(CurveError::TooClose(index));
                }
            }
        }
        let slopes = match shape {
            CurveShape::Linear => Vec::new(),
            CurveShape::Smooth => monotone_slopes(&points),
        };
        Ok(Self { shape, points, slopes })
    }

    /// The straight line from (0, 0) to (1, 1), which gives every value
    /// back as it is.
    pub fn identity() -> Self {
        Self {
            shape: CurveShape::Smooth,
            points: vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)],
            slopes: vec![1.0, 1.0],
        }
    }

    /// How the curve runs between its points.
    pub fn shape(&self) -> CurveShape {
        self.shape
    }

    /// The points, sorted by x.
    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// The value of the curve at `x`, from 0.0 to 1.0.
    ///
    /// Before the first point the curve holds its value, after the last
    /// point too. A NaN gives the value of the first point. Allocates
    /// nothing, so it can run on the audio thread.
    pub fn value_at(&self, x: f64) -> f64 {
        let first = self.points[0];
        let last = self.points[self.points.len() - 1];
        if x.is_nan() || x <= first.x {
            return first.y;
        }
        if x >= last.x {
            return last.y;
        }

        // The segment that holds x: from the last point at or before it to
        // the first point after it.
        let right = self.points.partition_point(|point| point.x <= x);
        let left = right - 1;
        let (a, b) = (self.points[left], self.points[right]);
        let width = b.x - a.x;
        let t = (x - a.x) / width;

        let y = match self.shape {
            CurveShape::Linear => a.y + t * (b.y - a.y),
            CurveShape::Smooth => {
                // Cubic Hermite between the two points with their slopes.
                let t2 = t * t;
                let t3 = t2 * t;
                let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
                let h10 = t3 - 2.0 * t2 + t;
                let h01 = -2.0 * t3 + 3.0 * t2;
                let h11 = t3 - t2;
                h00 * a.y
                    + h10 * width * self.slopes[left]
                    + h01 * b.y
                    + h11 * width * self.slopes[right]
            }
        };
        // The shapes stay between the values of their points. This only
        // catches rounding at the edges.
        y.clamp(0.0, 1.0)
    }
}

impl Default for Curve {
    fn default() -> Self {
        Self::identity()
    }
}

/// The slopes at the points for a monotone cubic through them, after
/// Fritsch and Carlson in the form of Fritsch and Butland: where the curve
/// turns, the slope is flat, elsewhere it is a weighted harmonic mean of the
/// slopes of the segments beside the point. At the two ends it follows the
/// three-point formula, limited so that the end segments do not overshoot.
fn monotone_slopes(points: &[CurvePoint]) -> Vec<f64> {
    let count = points.len();
    let widths: Vec<f64> = points.windows(2).map(|pair| pair[1].x - pair[0].x).collect();
    let secants: Vec<f64> = points
        .windows(2)
        .zip(&widths)
        .map(|(pair, width)| (pair[1].y - pair[0].y) / width)
        .collect();

    // Two points make a straight line.
    if count == 2 {
        return vec![secants[0]; 2];
    }

    let mut slopes = vec![0.0; count];
    for index in 1..count - 1 {
        let (before, after) = (secants[index - 1], secants[index]);
        if before * after > 0.0 {
            let (h0, h1) = (widths[index - 1], widths[index]);
            let w1 = 2.0 * h1 + h0;
            let w2 = h1 + 2.0 * h0;
            slopes[index] = (w1 + w2) / (w1 / before + w2 / after);
        }
    }
    slopes[0] = end_slope(widths[0], widths[1], secants[0], secants[1]);
    slopes[count - 1] = end_slope(
        widths[count - 2],
        widths[count - 3],
        secants[count - 2],
        secants[count - 3],
    );
    slopes
}

/// The slope at an end point from the end segment (`h0`, `d0`) and the one
/// next to it (`h1`, `d1`).
fn end_slope(h0: f64, h1: f64, d0: f64, d1: f64) -> f64 {
    let slope = ((2.0 * h0 + h1) * d0 - h0 * d1) / (h0 + h1);
    if slope * d0 <= 0.0 {
        0.0
    } else if d0 * d1 <= 0.0 && slope.abs() > 3.0 * d0.abs() {
        3.0 * d0
    } else {
        slope
    }
}

/// A curve as JSON has it, checked on its way to a [`Curve`].
#[derive(Serialize, Deserialize)]
struct CurveData {
    shape: CurveShape,
    points: Vec<CurvePoint>,
}

impl TryFrom<CurveData> for Curve {
    type Error = CurveError;

    fn try_from(data: CurveData) -> Result<Self, Self::Error> {
        Curve::new(data.shape, data.points)
    }
}

impl From<Curve> for CurveData {
    fn from(curve: Curve) -> Self {
        Self {
            shape: curve.shape,
            points: curve.points,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(shape: CurveShape, points: &[(f64, f64)]) -> Curve {
        let points = points.iter().map(|&(x, y)| CurvePoint::new(x, y)).collect();
        Curve::new(shape, points).expect("valid points")
    }

    /// The values at `steps + 1` evenly spaced x from 0 to 1.
    fn sampled(curve: &Curve, steps: usize) -> Vec<f64> {
        (0..=steps).map(|step| curve.value_at(step as f64 / steps as f64)).collect()
    }

    #[test]
    fn test_identity_gives_values_back() {
        let identity = Curve::identity();
        for x in [0.0, 0.1, 0.25, 0.5, 0.9, 1.0] {
            assert!((identity.value_at(x) - x).abs() < 1e-12, "{x}");
        }
        assert_eq!(Curve::default(), identity);
        // The same as a smooth curve made from its two points.
        assert_eq!(identity, curve(CurveShape::Smooth, &[(0.0, 0.0), (1.0, 1.0)]));
    }

    #[test]
    fn test_linear_runs_straight_between_points() {
        let linear = curve(CurveShape::Linear, &[(0.0, 0.0), (0.5, 0.8), (1.0, 1.0)]);
        assert_eq!(linear.value_at(0.25), 0.4);
        assert_eq!(linear.value_at(0.5), 0.8);
        assert!((linear.value_at(0.75) - 0.9).abs() < 1e-12);
    }

    #[test]
    fn test_values_beyond_the_points_hold() {
        for shape in [CurveShape::Linear, CurveShape::Smooth] {
            let inner = curve(shape, &[(0.2, 0.3), (0.6, 0.5), (0.8, 0.9)]);
            assert_eq!(inner.value_at(0.0), 0.3);
            assert_eq!(inner.value_at(0.1), 0.3);
            assert_eq!(inner.value_at(0.9), 0.9);
            assert_eq!(inner.value_at(1.0), 0.9);
            // Values outside 0 to 1 hold too.
            assert_eq!(inner.value_at(-1.0), 0.3);
            assert_eq!(inner.value_at(2.0), 0.9);
            assert_eq!(inner.value_at(f64::NAN), 0.3);
        }
    }

    #[test]
    fn test_smooth_runs_through_every_point() {
        let points = [(0.0, 0.1), (0.3, 0.2), (0.45, 0.8), (0.7, 0.6), (1.0, 1.0)];
        let smooth = curve(CurveShape::Smooth, &points);
        for (x, y) in points {
            assert!((smooth.value_at(x) - y).abs() < 1e-12, "{x}");
        }
    }

    #[test]
    fn test_smooth_does_not_overshoot_close_points() {
        // A steep step between two close points, where a natural spline
        // swings below the first and above the last.
        let points = [(0.0, 0.0), (0.48, 0.1), (0.52, 0.9), (1.0, 1.0)];
        let smooth = curve(CurveShape::Smooth, &points);
        let values = sampled(&smooth, 1000);
        for pair in values.windows(2) {
            assert!(pair[1] >= pair[0], "the curve falls: {} to {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn test_smooth_stays_between_the_values_of_each_segment() {
        let points = [(0.0, 0.5), (0.2, 1.0), (0.25, 0.0), (0.6, 0.3), (0.62, 0.3), (1.0, 0.0)];
        let smooth = curve(CurveShape::Smooth, &points);
        for pair in points.windows(2) {
            let (low, high) = (pair[0].1.min(pair[1].1), pair[0].1.max(pair[1].1));
            for step in 0..=100 {
                let x = pair[0].0 + (pair[1].0 - pair[0].0) * step as f64 / 100.0;
                let y = smooth.value_at(x);
                assert!((low - 1e-12..=high + 1e-12).contains(&y), "{y} at {x}");
            }
        }
        // Flat between two points at the same height.
        assert!((smooth.value_at(0.61) - 0.3).abs() < 1e-12);
    }

    #[test]
    fn test_smooth_is_smooth_at_a_point() {
        let smooth = curve(CurveShape::Smooth, &[(0.0, 0.0), (0.4, 0.7), (1.0, 1.0)]);
        let h = 1e-6;
        let before = (smooth.value_at(0.4) - smooth.value_at(0.4 - h)) / h;
        let after = (smooth.value_at(0.4 + h) - smooth.value_at(0.4)) / h;
        assert!((before - after).abs() < 1e-4, "{before} and {after}");
    }

    #[test]
    fn test_values_the_web_ui_checks() {
        // The same values as examples/midi-transform/webview/test/curve.test.ts,
        // which checks that the editor draws the curve the processor plays.
        let smooth = curve(CurveShape::Smooth, &[(0.0, 0.1), (0.3, 0.2), (0.45, 0.8), (0.7, 0.6), (1.0, 1.0)]);
        let floor = curve(CurveShape::Smooth, &[(0.2, 0.0), (0.5, 0.9), (1.0, 1.0)]);
        let expected = [
            (&smooth, 0.1, 0.11083158630328444),
            (&smooth, 0.35, 0.37064989517819696),
            (&smooth, 0.5, 0.7792000000000001),
            (&smooth, 0.65, 0.6208),
            (&smooth, 0.9, 0.7853198653198653),
            (&floor, 0.1, 0.0),
            (&floor, 0.3, 0.4043445692883894),
            (&floor, 0.45, 0.8474133895131085),
            (&floor, 0.75, 0.9752808988764046),
        ];
        for (curve, x, y) in expected {
            assert!((curve.value_at(x) - y).abs() < 1e-12, "{x}");
        }
    }

    #[test]
    fn test_points_that_make_no_curve_are_refused() {
        let new = |points: &[(f64, f64)]| {
            Curve::new(
                CurveShape::Smooth,
                points.iter().map(|&(x, y)| CurvePoint::new(x, y)).collect(),
            )
        };
        assert_eq!(new(&[]), Err(CurveError::TooFewPoints));
        assert_eq!(new(&[(0.5, 0.5)]), Err(CurveError::TooFewPoints));
        assert_eq!(new(&[(0.0, 0.0), (1.0, 1.5)]), Err(CurveError::OutOfRange(1)));
        assert_eq!(new(&[(-0.1, 0.0), (1.0, 1.0)]), Err(CurveError::OutOfRange(0)));
        assert_eq!(new(&[(0.0, f64::NAN), (1.0, 1.0)]), Err(CurveError::OutOfRange(0)));
        assert_eq!(new(&[(0.0, 0.0), (0.0, 1.0)]), Err(CurveError::NotIncreasing(1)));
        assert_eq!(
            new(&[(0.0, 0.0), (0.6, 0.5), (0.4, 0.6)]),
            Err(CurveError::NotIncreasing(2))
        );
        assert_eq!(new(&[(0.0, 0.0), (1e-310, 1.0)]), Err(CurveError::TooClose(1)));
    }

    #[test]
    fn test_the_closest_points_still_give_values() {
        let close = curve(CurveShape::Smooth, &[(0.0, 0.0), (Curve::MIN_POINT_DISTANCE, 1.0), (1.0, 0.5)]);
        for x in [Curve::MIN_POINT_DISTANCE / 2.0, 0.5, 1.0] {
            let y = close.value_at(x);
            assert!((0.0..=1.0).contains(&y), "{y} at {x}");
        }
    }

    #[test]
    fn test_json_holds_the_shape_and_the_points() {
        let linear = curve(CurveShape::Linear, &[(0.0, 0.0), (0.5, 0.75), (1.0, 1.0)]);
        let json = serde_json::to_string(&linear).unwrap();
        assert_eq!(
            json,
            r#"{"shape":"linear","points":[{"x":0.0,"y":0.0},{"x":0.5,"y":0.75},{"x":1.0,"y":1.0}]}"#
        );
        assert_eq!(serde_json::from_str::<Curve>(&json).unwrap(), linear);

        // The slopes come back with the points.
        let smooth = curve(CurveShape::Smooth, &[(0.0, 0.2), (0.3, 0.6), (1.0, 0.9)]);
        let json = serde_json::to_string(&smooth).unwrap();
        assert_eq!(serde_json::from_str::<Curve>(&json).unwrap(), smooth);
    }

    #[test]
    fn test_json_that_makes_no_curve_is_refused() {
        for json in [
            r#"{"shape":"smooth","points":[{"x":0.0,"y":0.0}]}"#,
            r#"{"shape":"smooth","points":[{"x":0.5,"y":0.0},{"x":0.2,"y":1.0}]}"#,
            r#"{"shape":"smooth","points":[{"x":0.0,"y":0.0},{"x":1.0,"y":2.0}]}"#,
            r#"{"shape":"bent","points":[{"x":0.0,"y":0.0},{"x":1.0,"y":1.0}]}"#,
        ] {
            assert!(serde_json::from_str::<Curve>(json).is_err(), "{json}");
        }
    }

    #[test]
    fn test_a_state_refuses_a_curve_that_does_not_fit() {
        let state = crate::State::<Curve>::default();
        let refused = state.load(br#"{"shape":"linear","points":[{"x":1.0,"y":0.0},{"x":0.0,"y":1.0}]}"#);
        assert!(refused.unwrap_err().contains("point 1"));
        assert_eq!(*state.get(), Curve::identity());
    }
}

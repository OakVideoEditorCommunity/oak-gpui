//! Frame-exact time types for the timeline widget.
//!
//! The timeline's canonical time unit is the **frame**, expressed as an
//! [`i64`] inside [`Frame`]. Durations and positions are never stored as
//! floating-point seconds: floats drift and accumulate rounding error, which
//! is unacceptable for an editing tool where an off-by-one frame is a visible
//! bug. The only place seconds appear is at the edges of the system —
//! converting wall-clock playback time into frames (see
//! [`super::playhead::PlayheadTicker`]) and formatting human-readable labels
//! via [`format_timecode`].
//!
//! This mirrors the Oak engine's time model, where canonical time is a
//! rational `int64 num/den` and the ABI exchanges `int64` frame timestamps
//! together with a per-sequence [`FrameRate`] such as `30000/1001`.
//!
//! # Invariants
//!
//! * A [`FrameRate`] is always normalized to a positive, non-zero numerator
//!   and denominator by [`FrameRate::new`].
//! * All conversions that produce frames from floats round deterministically;
//!   see [`seconds_to_frame`] for the exact rounding policy.
//! * [`FrameRange`] is half-open: `[start, end)`.

use crate::{Pixels, px};

/// A rational frame rate: `num / den` frames per second.
///
/// Frame rates in professional video are frequently *not* integers; the
/// classic example is NTSC 29.97 fps, which is exactly `30000/1001`. Storing
/// the rate as a pair of integers keeps every downstream computation exact.
///
/// # Invariants
///
/// Both `num` and `den` are guaranteed non-zero after construction through
/// [`FrameRate::new`]. Constructing the struct literal directly is possible
/// (the fields are public so the type is usable in `const` contexts) but
/// callers must uphold the non-zero invariant themselves.
///
/// # Examples
///
/// ```
/// # use gpui::timeline::FrameRate;
/// let ntsc = FrameRate::new(30000, 1001);
/// assert!((ntsc.as_f64() - 29.97002997).abs() < 1e-6);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRate {
    /// Numerator of the rate (frames per `den` seconds). Must be non-zero.
    pub num: u32,
    /// Denominator of the rate. Must be non-zero.
    pub den: u32,
}

impl FrameRate {
    /// NTSC "29.97" fps, exactly `30000/1001`.
    pub const NTSC_2997: FrameRate = FrameRate {
        num: 30000,
        den: 1001,
    };

    /// NTSC "23.976" fps, exactly `24000/1001`.
    pub const NTSC_23976: FrameRate = FrameRate {
        num: 24000,
        den: 1001,
    };

    /// Creates a frame rate from a numerator and denominator.
    ///
    /// # Panics
    ///
    /// Panics if either `num` or `den` is zero — a zero frame rate is
    /// meaningless and would cause division by zero in every conversion.
    ///
    /// # Examples
    ///
    /// ```
    /// # use gpui::timeline::FrameRate;
    /// let pal = FrameRate::new(25, 1);
    /// assert_eq!(pal.as_f64(), 25.0);
    /// ```
    pub fn new(num: u32, den: u32) -> Self {
        assert!(num != 0 && den != 0, "frame rate components must be non-zero");
        FrameRate { num, den }
    }

    /// Returns the rate as a floating-point frames-per-second value.
    ///
    /// Intended for display and for one-shot wall-clock conversions only;
    /// never store positions or durations derived from this value.
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

/// An absolute position or duration on the timeline, in frames.
///
/// Negative values are representable (they occasionally arise during drag
/// interactions before clamping) but are never valid as final positions in a
/// sequence; consumers should clamp to `[Frame(0), sequence_length)`.
///
/// `Frame` is `Ord`, so collections of frames (and of [`ClipId`](super::ClipId)s
/// keyed by frame) sort naturally.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Frame(pub i64);

impl Frame {
    /// Frame zero, the start of every sequence.
    pub const ZERO: Frame = Frame(0);

    /// Returns the raw frame number.
    pub fn number(self) -> i64 {
        self.0
    }

    /// Returns this frame as a [`Pixels`] offset at the given zoom
    /// (pixels per frame). Pure scaling, no scroll compensation — see
    /// [`TimelineState::point_at_frame`](super::TimelineState::point_at_frame)
    /// for the scroll-aware variant.
    pub fn to_pixels(self, zoom: f32) -> Pixels {
        Pixels::from(self.0 as f32 * zoom)
    }
}

impl std::ops::Add for Frame {
    type Output = Frame;

    fn add(self, rhs: Frame) -> Frame {
        Frame(self.0 + rhs.0)
    }
}

impl std::ops::Sub for Frame {
    type Output = Frame;

    fn sub(self, rhs: Frame) -> Frame {
        Frame(self.0 - rhs.0)
    }
}

impl std::ops::AddAssign for Frame {
    fn add_assign(&mut self, rhs: Frame) {
        self.0 += rhs.0;
    }
}

/// A half-open range of frames, `[start, end)`.
///
/// Half-open semantics match the Oak engine's block model: a clip occupying
/// frames `[10, 20)` has length 10 and abuts a clip starting at frame 20 with
/// no overlap and no gap. An empty range (`start == end`) is legal and
/// represents zero duration.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRange {
    /// First frame of the range (inclusive).
    pub start: Frame,
    /// End of the range (exclusive).
    pub end: Frame,
}

impl FrameRange {
    /// Creates a range from `start` (inclusive) to `end` (exclusive).
    ///
    /// # Panics
    ///
    /// Panics if `end < start`.
    pub fn new(start: Frame, end: Frame) -> Self {
        assert!(end >= start, "frame range end must not precede start");
        FrameRange { start, end }
    }

    /// The number of frames in the range. Zero for an empty range.
    pub fn len(&self) -> Frame {
        Frame(self.end.0 - self.start.0)
    }

    /// Whether the range contains no frames.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Whether `frame` lies inside the range (`start <= frame < end`).
    pub fn contains(&self, frame: Frame) -> bool {
        self.start <= frame && frame < self.end
    }

    /// Whether two ranges share at least one frame.
    pub fn overlaps(&self, other: &FrameRange) -> bool {
        self.start < other.end && other.start < self.end
    }
}

/// How time should be presented to the user in rulers and inspectors.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeDisplay {
    /// `HH:MM:SS:FF` timecode. This is the default and the standard in
    /// professional video editing.
    ///
    /// Drop-frame timecode (`;` separator, frame-number skipping for NTSC
    /// rates) is **not implemented yet**; [`format_timecode`] currently
    /// always produces non-drop-frame timecode. See its documentation.
    #[default]
    Timecode,
    /// A plain frame counter, e.g. `1048576`.
    Frames,
    /// Seconds with millisecond precision, e.g. `83.708`.
    Seconds,
}

/// Converts a frame position to floating-point seconds at `rate`.
///
/// This is a pure, one-shot conversion: `frame * den / num`. It is exact for
/// any frame whose magnitude keeps `frame * den` within `f64`'s 53-bit
/// mantissa (far beyond any realistic sequence length).
///
/// # Examples
///
/// ```
/// # use gpui::timeline::{Frame, FrameRate, frame_to_seconds};
/// let rate = FrameRate::new(30000, 1001);
/// // Frame 30 at ~29.97 fps is just over one second.
/// assert!((frame_to_seconds(Frame(30), rate) - 1.001).abs() < 1e-9);
/// ```
pub fn frame_to_seconds(frame: Frame, rate: FrameRate) -> f64 {
    frame.0 as f64 * rate.den as f64 / rate.num as f64
}

/// Converts floating-point seconds to the nearest frame at `rate`.
///
/// # Rounding policy
///
/// The result is rounded to the **nearest frame, ties away from zero**
/// (`f64::round` semantics). This makes [`seconds_to_frame`] and
/// [`frame_to_seconds`] approximate inverses for any input that is already
/// near a frame boundary, and it is deterministic across platforms.
///
/// This function exists for converting wall-clock durations (mouse drags
/// measured in seconds, playback elapsed time) into frames. It must never be
/// used to *store* time — convert once, then keep the [`Frame`].
///
/// # Examples
///
/// ```
/// # use gpui::timeline::{Frame, FrameRate, seconds_to_frame};
/// let rate = FrameRate::new(24, 1);
/// assert_eq!(seconds_to_frame(rate, 1.0), Frame(24));
/// assert_eq!(seconds_to_frame(rate, 1.02), Frame(24)); // rounds to nearest
/// assert_eq!(seconds_to_frame(rate, 1.03), Frame(25));
/// ```
pub fn seconds_to_frame(rate: FrameRate, seconds: f64) -> Frame {
    Frame((seconds * rate.num as f64 / rate.den as f64).round() as i64)
}

/// Formats `frame` for display according to `display`.
///
/// * [`TimeDisplay::Frames`] — the raw frame number.
/// * [`TimeDisplay::Seconds`] — seconds with millisecond precision.
/// * [`TimeDisplay::Timecode`] — non-drop-frame `HH:MM:SS:FF`. The frame
///   component width derives from the frame rate (two digits for rates below
///   100 fps).
///
/// # Drop-frame timecode
///
/// Drop-frame timecode (the `HH:MM:SS;FF` convention used with NTSC rates so
/// that timecode stays in lockstep with wall-clock time) is **future work**.
/// Calling this with [`TimeDisplay::Timecode`] and an NTSC rate such as
/// [`FrameRate::NTSC_2997`] currently yields non-drop-frame timecode, which
/// drifts from wall-clock time by about 3.6 seconds per hour. Callers that
/// need broadcast-correct labels must not rely on this function yet.
///
/// Negative frames are formatted with a leading `-` applied to the whole
/// timecode (e.g. `-00:00:01:12`).
///
/// # Examples
///
/// ```
/// # use gpui::timeline::{Frame, FrameRate, TimeDisplay, format_timecode};
/// let rate = FrameRate::new(24, 1);
/// assert_eq!(format_timecode(Frame(0), rate, TimeDisplay::Timecode), "00:00:00:00");
/// assert_eq!(
///     format_timecode(Frame(24 * 60 * 60 + 24 * 60 + 24 + 12), rate, TimeDisplay::Timecode),
///     "01:01:01:12",
/// );
/// assert_eq!(format_timecode(Frame(42), rate, TimeDisplay::Frames), "42");
/// ```
pub fn format_timecode(frame: Frame, rate: FrameRate, display: TimeDisplay) -> String {
    match display {
        TimeDisplay::Frames => frame.0.to_string(),
        TimeDisplay::Seconds => format!("{:.3}", frame_to_seconds(frame, rate)),
        TimeDisplay::Timecode => {
            let negative = frame.0 < 0;
            let mut n = frame.0.unsigned_abs();
            // Nominal (integer) frame count per second, matching the
            // non-drop-frame convention: NTSC 29.97 uses 30 frames/sec.
            let fps = rate.as_f64().round() as u64;
            let frames = n % fps;
            n /= fps;
            let seconds = n % 60;
            n /= 60;
            let minutes = n % 60;
            let hours = n / 60;
            format!(
                "{}{:02}:{:02}:{:02}:{:02}",
                if negative { "-" } else { "" },
                hours,
                minutes,
                seconds,
                frames
            )
        }
    }
}

/// What produced a [`SnapPoint`]. Used by the UI to pick an indicator style
/// and by tests to assert snapping priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SnapKind {
    /// The start edge of a clip.
    ClipStart,
    /// The end edge of a clip.
    ClipEnd,
    /// The playhead.
    Playhead,
    /// An edge of the work area (in or out point).
    WorkAreaEdge,
    /// A user or chapter marker.
    Marker,
}

/// A frame position that dragged elements can snap to.
///
/// Snap points are gathered fresh on every drag move from the current
/// [`TimelineDataSource`](super::TimelineDataSource): clip edges, the
/// playhead, work-area edges and markers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SnapPoint {
    /// The frame to snap to.
    pub frame: Frame,
    /// What this point represents.
    pub kind: SnapKind,
}

/// The outcome of a successful [`snap`] query.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnapResult {
    /// The snapped frame — equal to the winning [`SnapPoint`]'s frame.
    pub frame: Frame,
    /// The kind of the winning snap point.
    pub kind: SnapKind,
    /// On-screen distance between the drag position and the snap point, in
    /// pixels. Always `<=` the threshold passed to [`snap`]. Useful for
    /// fading the snap indicator as the cursor approaches.
    pub distance: Pixels,
}

/// Finds the best snap target for a dragged position, or `None` if nothing
/// is close enough.
///
/// # Contract
///
/// * `target` is the unsnapped frame position of the dragged edge or clip.
/// * `points` is evaluated lazily; the iterator may be cheap to reconstruct
///   per mouse-move, so implementations should consume it exactly once.
/// * `threshold_px` is the maximum on-screen distance at which snapping
///   engages, converted to frames internally via `zoom` (pixels per frame).
///   A threshold of `0` disables snapping.
/// * When several points are within range, the **nearest on screen** wins.
///   Ties are broken by the **earlier frame** first, then by [`SnapKind`]
///   priority — `Playhead` first, then `ClipStart`/`ClipEnd`, then
///   `WorkAreaEdge`, then `Marker` — so the behavior is deterministic
///   regardless of iterator order.
///
/// The returned frame is always exactly one of the provided snap points'
/// frames; this function never invents intermediate positions.
pub fn snap(
    target: Frame,
    points: impl Iterator<Item = SnapPoint>,
    threshold_px: Pixels,
    zoom: f32,
) -> Option<SnapResult> {
    if threshold_px.0 <= 0.0 || zoom <= 0.0 {
        return None;
    }

    // Work in frame space for the distance comparison: a threshold given in
    // pixels is `threshold_px / zoom` frames at this zoom, and a point's
    // on-screen distance is `|point - target| * zoom`. Both quantities scale
    // by the same positive zoom, so ordering is preserved — we compare in
    // frame space and only convert the winning distance back to pixels.
    let threshold_frames = threshold_px.0 / zoom;

    // Priority per [`SnapKind`] for deterministic tie-breaking: `Playhead`
    // first, then `ClipStart`/`ClipEnd`, then `WorkAreaEdge`, then `Marker`.
    let kind_rank = |kind: SnapKind| match kind {
        SnapKind::Playhead => 0,
        SnapKind::ClipStart | SnapKind::ClipEnd => 1,
        SnapKind::WorkAreaEdge => 2,
        SnapKind::Marker => 3,
    };

    // Best candidate, compared lexicographically: (screen distance in frames,
    // frame number, kind rank). Smaller is better on every component. The
    // frame-number tiebreak prefers the earlier snap point when two points are
    // equally close (so the result is always exactly one of the given frames,
    // never an interpolated position), and the kind rank breaks ties between
    // points sharing a frame; both keep the outcome independent of iterator
    // order.
    let mut best: Option<(f32, i64, u8)> = None;
    let mut best_kind = SnapKind::Marker;
    for point in points {
        let dist = (point.frame.0 - target.0).unsigned_abs() as f32;
        if dist > threshold_frames {
            continue;
        }
        let candidate = (dist, point.frame.0, kind_rank(point.kind));
        if best.map_or(true, |current| candidate < current) {
            best = Some(candidate);
            best_kind = point.kind;
        }
    }

    best.map(|(dist, frame, _)| SnapResult {
        frame: Frame(frame),
        kind: best_kind,
        distance: px(dist * zoom),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_range_is_half_open() {
        let range = FrameRange::new(Frame(10), Frame(20));
        assert_eq!(range.len(), Frame(10));
        assert!(range.contains(Frame(10)));
        assert!(!range.contains(Frame(20)));
        assert!(range.overlaps(&FrameRange::new(Frame(19), Frame(30))));
        assert!(!range.overlaps(&FrameRange::new(Frame(20), Frame(30))));
    }

    #[test]
    fn seconds_round_trips_to_nearest_frame() {
        let rate = FrameRate::NTSC_2997;
        let frame = seconds_to_frame(rate, 10.0);
        assert!((frame_to_seconds(frame, rate) - 10.0).abs() < 0.02);
    }

    #[test]
    fn snap_disabled_by_zero_threshold() {
        let points = [SnapPoint {
            frame: Frame(10),
            kind: SnapKind::Playhead,
        }];
        assert!(snap(Frame(12), points.into_iter(), px(0.0), 1.0).is_none());
    }

    #[test]
    fn snap_requires_positive_zoom() {
        let points = [SnapPoint {
            frame: Frame(10),
            kind: SnapKind::Playhead,
        }];
        assert!(snap(Frame(10), points.into_iter(), px(10.0), 0.0).is_none());
    }

    #[test]
    fn snap_returns_none_when_nothing_is_within_range() {
        let points = [SnapPoint {
            frame: Frame(100),
            kind: SnapKind::Playhead,
        }];
        // At zoom 1.0 a 5-px threshold is 5 frames; target 92 is 8 frames away.
        assert!(snap(Frame(92), points.into_iter(), px(5.0), 1.0).is_none());
    }

    #[test]
    fn snap_prefers_the_nearest_point() {
        let points = [
            SnapPoint {
                frame: Frame(90),
                kind: SnapKind::Marker,
            },
            SnapPoint {
                frame: Frame(95),
                kind: SnapKind::Marker,
            },
        ];
        let result = snap(Frame(92), points.into_iter(), px(50.0), 1.0).unwrap();
        assert_eq!(result.frame, Frame(90));
        assert_eq!(result.kind, SnapKind::Marker);
        assert_eq!(result.distance, px(2.0));
    }

    #[test]
    fn snap_ties_break_by_kind_priority_regardless_of_iteration_order() {
        let priority_points = [
            SnapPoint {
                frame: Frame(100),
                kind: SnapKind::Playhead,
            },
            SnapPoint {
                frame: Frame(100),
                kind: SnapKind::ClipStart,
            },
        ];
        // Equal distance; higher-priority kind wins even though it appears
        // first in the iterator.
        let result = snap(Frame(100), priority_points.into_iter(), px(10.0), 1.0).unwrap();
        assert_eq!(result.kind, SnapKind::Playhead);
        assert_eq!(result.frame, Frame(100));

        // Reversed iteration order changes nothing.
        let reversed = [
            SnapPoint {
                frame: Frame(100),
                kind: SnapKind::ClipStart,
            },
            SnapPoint {
                frame: Frame(100),
                kind: SnapKind::Playhead,
            },
        ];
        let result = snap(Frame(100), reversed.into_iter(), px(10.0), 1.0).unwrap();
        assert_eq!(result.kind, SnapKind::Playhead);
    }

    #[test]
    fn snap_result_frame_is_always_a_snap_point_frame() {
        // Two points at equal distance on opposite sides; the earlier frame
        // wins via the frame-number tiebreak, never an interpolated position.
        let points = [
            SnapPoint {
                frame: Frame(98),
                kind: SnapKind::WorkAreaEdge,
            },
            SnapPoint {
                frame: Frame(102),
                kind: SnapKind::ClipEnd,
            },
        ];
        let result = snap(Frame(100), points.into_iter(), px(10.0), 1.0).unwrap();
        assert_eq!(result.frame, Frame(98));
        assert_eq!(result.distance, px(2.0));
    }

    #[test]
    fn snap_threshold_scales_with_zoom() {
        // At zoom 2.0 the same 10-px threshold covers only 5 frames.
        let points = [SnapPoint {
            frame: Frame(50),
            kind: SnapKind::ClipEnd,
        }];
        assert!(snap(Frame(56), points.into_iter(), px(10.0), 2.0).is_none());
        let result = snap(Frame(55), points.into_iter(), px(10.0), 2.0).unwrap();
        assert_eq!(result.frame, Frame(50));
        assert_eq!(result.distance, px(10.0));
    }
}

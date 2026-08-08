//! Data-source traits: how the timeline widget reads your model.
//!
//! The timeline is **data-agnostic**: it owns no clips, tracks, or sequence
//! state. Instead the host application implements [`TimelineDataSource`] (and
//! the [`TrackData`] / [`ClipData`] traits it pulls in) over its own model,
//! and the widget re-reads through those traits every time the model entity
//! notifies.
//!
//! # Mapping to Oak engine concepts
//!
//! These traits are deliberately shaped like Oak's engine model so the
//! adapter is a thin, mechanical translation:
//!
//! | Timeline trait concept            | Oak engine concept                        |
//! |-----------------------------------|-------------------------------------------|
//! | [`TrackData::clips`]              | A track's block list                      |
//! | [`ClipData`] (normal clip)        | `ClipBlock`                               |
//! | [`ClipData`] with empty range     | `GapBlock` (never emitted — gaps are the absence of clips) |
//! | [`ClipData::in_transition`] etc.  | `TransitionBlock` attached to a clip edge |
//! | [`TrackData::kind`]               | Track type `k_video` / `k_audio` / `k_subtitle` |
//! | [`TrackData::is_locked`] etc.     | Track lock / mute / solo / show flags     |
//! | [`ClipData::media_in`]            | Clip `media_in` (source offset)           |
//! | [`ClipData::linked_ids`]          | Linked clips (e.g. audio+video from one source) |
//! | [`TimelineDataSource::frame_rate`] | Per-sequence frame rate (e.g. 30000/1001) |
//!
//! # Consistency requirements
//!
//! The widget assumes, but cannot enforce, that within a single read:
//!
//! * [`ClipId`]s are unique across the whole data source and stable across
//!   frames (they key the selection set and drag state).
//! * Clips within one track do not overlap and are returned in ascending
//!   frame order.
//! * Values never change except as observed between `cx.notify()` calls —
//!   the widget may cache layout between notifications.

use crate::{Hsla, Pixels, SharedString};

use super::time::{Frame, FrameRange, FrameRate};

/// A stable, unique identifier for a clip within a [`TimelineDataSource`].
///
/// The widget treats these as opaque: they key the selection set, appear in
/// edit-request events, and are passed back to [`ClipData`] providers. The
/// host application chooses the mapping (Oak will use its engine's clip
/// pointers/UUIDs hashed down, or a generational index).
///
/// Ordered so the selection set can be a `BTreeSet`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClipId(pub u64);

/// The kind of content a track holds.
///
/// Maps directly onto Oak's track types (`k_video`, `k_audio`,
/// `k_subtitle`). The kind drives default track colors, which toggle buttons
/// the header shows (audio tracks get *mute*, video tracks get *show*), and
/// which clip decorations (waveform vs. thumbnails) are offered.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrackKind {
    /// Video track. Stacks visually above/below siblings by compositing
    /// order; upper tracks occlude lower ones.
    #[default]
    Video,
    /// Audio track. Contributes to the mix; subject to mute/solo.
    Audio,
    /// Subtitle / caption track.
    Subtitle,
}

/// A single clip on a track.
///
/// Corresponds to Oak's `ClipBlock`. All frames are in **sequence time**
/// (frames on the timeline), except [`ClipData::media_in`], which is in
/// **source media time**.
///
/// Implementations must be cheap to call repeatedly during paint; anything
/// expensive (thumbnails, waveforms) belongs behind the
/// [`ClipDecorator`](super::ClipDecorator) cache hooks, not here.
pub trait ClipData {
    /// The clip's stable, unique [`ClipId`]. See its docs for the stability
    /// requirements.
    fn id(&self) -> ClipId;

    /// The clip's occupied range in sequence time, `[start, end)`.
    ///
    /// Must have non-zero length for a real clip. Gaps between clips are not
    /// represented (Oak's `GapBlock` is implicit here).
    fn range(&self) -> FrameRange;

    /// The offset into the source media, in frames, at which this clip
    /// starts playing.
    ///
    /// A clip created from frame 100 of a source file reports `Frame(100)`.
    /// Trimming the clip's left edge by `n` frames increases this by `n`.
    /// The widget displays this nowhere directly but forwards it in trim
    /// requests' docs and uses it for thumbnail/waveform alignment via the
    /// decorator hooks.
    fn media_in(&self) -> Frame;

    /// Short label shown on the clip body (typically the source file name).
    fn label(&self) -> SharedString;

    /// Base color of the clip body. The widget derives hover/selected/
    /// disabled shades from it. `None` falls back to the track-kind default.
    fn color(&self) -> Option<Hsla> {
        None
    }

    /// Clips that must move and trim together with this one.
    ///
    /// This is Oak's *linked clips* concept: audio and video clips recorded
    /// from the same source are linked, so trimming the video's head trims
    /// the audio identically. The widget expands every move/trim request to
    /// cover the transitive link group before emitting it — see
    /// [`TimelineEvent::ClipMoveRequested`](super::TimelineEvent::ClipMoveRequested).
    ///
    /// Must not contain `self.id()`. May be empty (the common case).
    fn linked_ids(&self) -> Vec<ClipId> {
        Vec::new()
    }

    /// Duration of the clip's **in transition** (Oak: the `TransitionBlock`
    /// attached to the clip's head), if any, in frames.
    ///
    /// Rendered as a wedge at the clip's left edge. The transition itself is
    /// edited elsewhere; the timeline only displays it.
    fn in_transition(&self) -> Option<Frame> {
        None
    }

    /// Duration of the clip's **out transition** (Oak: the `TransitionBlock`
    /// attached to the clip's tail), if any, in frames.
    fn out_transition(&self) -> Option<Frame> {
        None
    }

    /// Whether the clip is enabled (not disabled/bypassed).
    ///
    /// Disabled clips render dimmed and are skipped by snapping; the flag
    /// itself is toggled through the app's engine, not the timeline.
    fn is_enabled(&self) -> bool {
        true
    }
}

/// A single track (row) of the timeline.
///
/// Corresponds to an Oak track of one of the `k_video` / `k_audio` /
/// `k_subtitle` types.
pub trait TrackData {
    /// The clip type carried by this track.
    type Clip: ClipData;

    /// What kind of content this track holds.
    fn kind(&self) -> TrackKind;

    /// Display name for the track header (e.g. `V1`, `Music`).
    fn name(&self) -> SharedString;

    /// Whether the track is locked. Locked tracks render normally but reject
    /// all edit gestures (no moves, no trims, no drops); the widget checks
    /// this before emitting any edit request.
    fn is_locked(&self) -> bool {
        false
    }

    /// Whether the track is muted (audio) — silenced in playback.
    ///
    /// Meaningful for [`TrackKind::Audio`]; the header only shows the mute
    /// button there.
    fn is_muted(&self) -> bool {
        false
    }

    /// Whether the track is soloed (audio) — all non-solo tracks are
    /// temporarily silenced.
    fn is_solo(&self) -> bool {
        false
    }

    /// Whether the track is visible (video) — Oak's *show* flag.
    ///
    /// Meaningful for [`TrackKind::Video`] and [`TrackKind::Subtitle`].
    fn is_visible(&self) -> bool {
        true
    }

    /// The track's row height in the clip area.
    ///
    /// This is view state that Oak persists per sequence; it changes via
    /// [`TimelineEvent::TrackHeightChanged`](super::TimelineEvent::TrackHeightChanged)
    /// and must be written back into the model there.
    fn height(&self) -> Pixels;

    /// The clips on this track, in ascending frame order, non-overlapping.
    ///
    /// Returned as a slice so the widget can binary-search by frame. If your
    /// model cannot produce a contiguous slice, collect into a buffer you
    /// own and return that.
    fn clips(&self) -> &[Self::Clip];
}

/// A marker on the sequence ruler (chapter marks, annotations).
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    /// Where the marker sits, in sequence frames.
    pub frame: Frame,
    /// Label shown in the marker tooltip / ruler.
    pub label: SharedString,
    /// Optional marker color; defaults to the theme's accent.
    pub color: Option<Hsla>,
}

/// The root data source the timeline widget reads from.
///
/// Implement this on the model object your app already holds as an
/// [`Entity`](crate::Entity), and hand that entity to
/// [`TimelineView::new`](super::TimelineView::new). The widget observes the
/// entity and re-reads everything through this trait on `cx.notify()`.
///
/// # Wiring into Oak
///
/// In Oak, this trait is implemented on the facade over the current
/// sequence: `frame_rate` and `sequence_length` come from the sequence
/// header, `track` walks the sequence's track list, and every edit the
/// widget requests arrives as a [`TimelineEvent`](super::TimelineEvent) that
/// the facade turns into an undoable engine command.
pub trait TimelineDataSource: 'static {
    /// The track type returned by [`TimelineDataSource::track`].
    type Track: TrackData;

    /// The sequence's frame rate (e.g. [`FrameRate::NTSC_2997`]).
    ///
    /// Assumed constant for the lifetime of the sequence; changing it
    /// requires rebuilding the view.
    fn frame_rate(&self) -> FrameRate;

    /// Total length of the sequence in frames — the position just past the
    /// last frame of content. Playhead and scroll are clamped to this.
    fn sequence_length(&self) -> Frame;

    /// Number of tracks. Indices are stable within a single notification
    /// cycle.
    fn track_count(&self) -> usize;

    /// The track at `index`, or `None` if out of range.
    ///
    /// Returns by value so implementations can hand out lightweight
    /// snapshot views of their internal track storage.
    fn track(&self, index: usize) -> Option<Self::Track>;

    /// All sequence markers, in ascending frame order.
    fn markers(&self) -> Vec<Marker> {
        Vec::new()
    }
}

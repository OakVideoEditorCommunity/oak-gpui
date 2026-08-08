//! Video-editing timeline widget: tracks, clips, ruler, playhead, snapping.
//!
//! This module provides the timeline at the heart of a non-linear video
//! editor (NLE): a ruler with timecode, stacked tracks of clips, a playhead,
//! marquee selection, drag-to-move and drag-to-trim gestures with snapping.
//!
//! # Architecture
//!
//! The widget is **data-agnostic**. It owns no sequence model; instead the
//! host application implements the traits in [`data`](crate::timeline::data) — [`TimelineDataSource`](crate::timeline::TimelineDataSource),
//! [`TrackData`](crate::timeline::TrackData), [`ClipData`](crate::timeline::ClipData) — over its own model and hands the view an
//! [`Entity`](crate::Entity) of that implementation. The view re-reads
//! everything through those traits whenever the entity notifies.
//!
//! **All edits are emitted as *requests*.** The widget never mutates the
//! model. Every gesture (move, trim, toggle, resize) ends by emitting a
//! [`TimelineEvent`](crate::timeline::TimelineEvent); the host applies the request through its engine and
//! undo stack — keeping the engine the single source of truth — and then
//! calls `cx.notify()` on the data-source entity so the view repaints. If
//! the engine rejects an edit, the host simply doesn't notify and the
//! gesture has no visible effect.
//!
//! **Time is frame-exact.** All positions and durations are integer
//! [`Frame`](crate::timeline::Frame)s at a rational [`FrameRate`](crate::timeline::FrameRate) (e.g. `30000/1001` for NTSC
//! 29.97). No float seconds appear in the public API, so repeated edits
//! cannot accumulate rounding drift; see [`time`](crate::timeline::time).
//!
//! # Wiring into Oak
//!
//! Oak (the Olive-fork video editor built on this crate) maps these pieces
//! onto its `oakengine` facade as follows:
//!
//! * [`TimelineDataSource`](crate::timeline::TimelineDataSource) is implemented on the facade's snapshot of the
//!   current sequence: frame rate and length from the sequence header,
//!   tracks from the track list (`k_video` / `k_audio` / `k_subtitle` →
//!   [`TrackKind`](crate::timeline::TrackKind)), clips from each track's block list (`ClipBlock` →
//!   [`ClipData`](crate::timeline::ClipData); `GapBlock` is implicit, `TransitionBlock` →
//!   [`ClipData::in_transition`](crate::timeline::ClipData::in_transition) / [`ClipData::out_transition`](crate::timeline::ClipData::out_transition)).
//! * Each [`TimelineEvent`](crate::timeline::TimelineEvent) becomes an undoable engine command:
//!   `ClipMoveRequested` → `move_clip`, `ClipTrimRequested` → `trim_clip`
//!   (both expanded to the clip's linked group), header toggles → track
//!   flag setters. After applying, the facade notifies the sequence entity.
//! * [`ClipDecorator`](crate::timeline::ClipDecorator) is implemented over Oak's codec frame cache and audio
//!   peak cache to paint thumbnails and waveforms.
//! * [`PlayheadTicker`](crate::timeline::PlayheadTicker) is driven by the engine's playback state; seek
//!   requests flow back as non-undoable `set_playhead` calls.
//!
//! See `examples/learn/timeline.rs` for a minimal working sketch with a mock
//! data source.
//!
//! # Status
//!
//! Implemented: the pure time/state arithmetic ([`time`](crate::timeline::time),
//! [`TimelineState`](crate::timeline::TimelineState)) is unit-tested, and
//! [`TimelineView`](crate::timeline::TimelineView) composes the ruler, track
//! headers, clips and playhead into the layout documented above, wiring up
//! seek, clip move/trim with snapping, marquee selection, zoom and
//! track-height resize.

pub mod clip;
pub mod data;
pub mod playhead;
pub mod ruler;
pub mod state;
pub mod time;
pub mod track_header;
pub mod timeline_view;

pub use clip::*;
pub use data::*;
pub use playhead::*;
pub use ruler::*;
pub use state::*;
pub use time::*;
pub use track_header::*;
pub use timeline_view::*;

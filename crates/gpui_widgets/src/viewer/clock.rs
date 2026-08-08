//! The playback clock a [`ViewerWidget`](super::ViewerWidget) reads its
//! position from.
//!
//! The host implements this trait over its engine (in Oak: the audio engine
//! clock queried through the C ABI). The widget polls it on a timer and only
//! *requests* transport changes; the engine is the single source of truth.

use gpui::timeline::{Frame, FrameRate};

/// A read-only view of the engine's playback clock.
pub trait PlaybackClock: 'static {
    /// The current playhead frame.
    fn current_frame(&self) -> Frame;
    /// Whether playback is running.
    fn is_playing(&self) -> bool;
    /// The sequence's frame rate.
    fn frame_rate(&self) -> FrameRate;
}

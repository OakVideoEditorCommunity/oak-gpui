//! Linear effect-stack inspector widget.
//!
//! The effect stack presents the processing chain for a single clip (or the
//! current selection) as a vertical, linear list of cards — e.g.
//! `Media → Transform → OCIO LUT → Output`. It is the inspector-style
//! counterpart to [`crate::node_graph`]: **both widgets are views of the same
//! underlying node data** ("two views of one model").
//!
//! # Architecture
//!
//! - **Trait-driven data source.** The widget never owns the document. The
//!   app implements [`EffectStackDataSource`](crate::effect_stack::EffectStackDataSource) (and [`EffectData`](crate::effect_stack::EffectData) for each
//!   card) on an [`Entity`](crate::Entity)-backed model and hands it to
//!   [`EffectStackView`](crate::effect_stack::EffectStackView). The view reads through the trait on every render,
//!   so the app's engine remains the single source of truth.
//! - **Edits are requests, not mutations.** Every user gesture (toggle,
//!   reorder, remove, add, context menu) is surfaced as an
//!   [`EffectStackEvent`](crate::effect_stack::EffectStackEvent) via [`EventEmitter`](crate::EventEmitter). The view
//!   never mutates the data source itself. The app applies the request
//!   through its engine and undo stack, then calls
//!   [`cx.notify()`](crate::Context::notify) on the data source (or the view)
//!   so the stack re-renders. Optimistic in-view state is intentionally
//!   limited to transient visuals (e.g. the drag insertion indicator).
//!
//! # Relationship to `crate::node_graph`
//!
//! The stack shows the *linear path* through the node graph for one clip:
//! source node at the top, output node at the bottom, and every effect node
//! on the path in signal order. Mapping gestures back onto the graph is the
//! **app's responsibility**:
//!
//! - **Reorder a card** = detach the effect node from its neighbors and
//!   rewire the path (previous node's output → moved node → node that used to
//!   follow the insertion point).
//! - **Remove a card** = delete the node and bridge the gap.
//! - **Add a card** = insert a new node at the requested path position.
//! - **Enable toggle** = the node's bypass flag.
//!
//! When both the stack and the node graph are visible at the same time, both
//! should observe the same document entity so that an edit in one is
//! reflected in the other after `cx.notify()`.
//!
//! # Wiring into Oak
//!
//! In Oak (the video editor this widget is built for), the intended wiring
//! is:
//!
//! 1. `oakengine` owns the node graph and the undo stack. Oak implements
//!    [`EffectStackDataSource`](crate::effect_stack::EffectStackDataSource) over a view-model entity that derives the
//!    ordered effect list from the graph path of the selected clip.
//! 2. Oak subscribes to the [`Entity<EffectStackView<_>>`](crate::Entity)
//!    and, for each [`EffectStackEvent`](crate::effect_stack::EffectStackEvent), builds the matching engine command,
//!    pushes it onto the undo stack, executes it, and calls `cx.notify()`.
//! 3. Parameter UIs (per-effect controls) are supplied through
//!    [`EffectStackView::params_renderer`](crate::effect_stack::EffectStackView::params_renderer) and live inside the expanded card
//!    body. When a parameter edit happens, Oak calls
//!    [`EffectStackView::notify_parameter_changed`](crate::effect_stack::EffectStackView::notify_parameter_changed) so the view can refresh
//!    badges and so subscribers can schedule re-rendering.
//! 4. Selection changes (which clip is active) are pushed into the
//!    view-model; the stack's empty state renders automatically when
//!    [`EffectStackDataSource::target_label`](crate::effect_stack::EffectStackDataSource::target_label) returns `None`.
//!
//! # Modules
//!
//! - [`data`](crate::effect_stack::data): the data-source traits and card metadata types.
//! - [`stack_view`](crate::effect_stack::stack_view): the top-level [`EffectStackView`](crate::effect_stack::EffectStackView) and
//!   [`EffectStackEvent`](crate::effect_stack::EffectStackEvent).
//! - [`card`](crate::effect_stack::card): the [`EffectCard`](crate::effect_stack::EffectCard) component and the drop-position
//!   [`InsertIndicator`](crate::effect_stack::card::InsertIndicator).

pub mod card;
pub mod data;
pub mod stack_view;

pub use card::*;
pub use data::*;
pub use stack_view::*;

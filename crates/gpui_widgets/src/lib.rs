//! Widget library built on gpui for the Oak video editor: form controls,
//! menus and dialogs, and NLE-specific components (viewer, scopes, project
//! explorer).
//!
//! # Architecture
//!
//! Widgets follow the same contract as the `gpui::timeline` /
//! `gpui::node_graph` / `gpui::effect_stack` / `gpui::dock` modules:
//!
//! * **Data-agnostic** — a widget owns no model. The host implements a
//!   data-source trait over its own state and hands the widget an
//!   [`Entity`](gpui::Entity) of that implementation.
//! * **Requests only** — every edit is emitted as a request event through
//!   `cx.emit`; the host applies it through its engine (the single source of
//!   truth) and then notifies the data entity.
//! * **Testable core** — pure value/geometry/state-machine logic lives in
//!   files with no gpui coupling (e.g. [`value`], [`slider::model`]) and is
//!   covered by plain unit tests.

pub mod checkbox;
pub mod color;
pub mod combo_box;
pub mod keyable;
pub mod radio_group;
pub mod slider;
pub mod spinbox;
pub mod value;

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

pub mod audio_meter;
pub mod checkbox;
pub mod color;
pub mod combo_box;
pub mod curve_editor;
pub mod dialog;
pub mod keyable;
pub mod menu;
pub mod project_explorer;
pub mod radio_group;
pub mod scopes;
pub mod slider;
pub mod spinbox;
pub mod theme;
pub mod value;
pub mod viewer;

#[cfg(test)]
mod tests {
    use gpui::prelude::*;
    use gpui::{AbsoluteLength, DefiniteLength, div, rems};

    /// Fractional spacing helpers exist on the [`Styled`](gpui::Styled) trait
    /// (via `gpui_macros::padding_style_methods!` / `margin_style_methods!`),
    /// named after Tailwind's fractional scale: `0p5`, `1p5`, `2p5`, `3p5`
    /// (i.e. 0.5/1.5/2.5/3.5 units of 4px = 2/6/10/14px).
    ///
    /// We deliberately do **not** add `_0_5`-style aliases (`py_0_5`,
    /// `px_1_5`, ...): every fractional value they would name already exists
    /// under the established `0p5`/`1p5`/`2p5`/`3p5` convention used
    /// throughout gpui and gpui_widgets, and a second naming scheme for the
    /// same helpers would only fragment the API surface. This test pins the
    /// helpers (and their values) so a future refactor of the style macros
    /// cannot silently drop them.
    #[test]
    fn fractional_spacing_helpers_exist() {
        let mut padding = div().py_0p5().px_1p5();
        assert_eq!(
            padding.style().padding.top,
            Some(DefiniteLength::Absolute(AbsoluteLength::Rems(rems(0.125))))
        );
        assert_eq!(
            padding.style().padding.left,
            Some(DefiniteLength::Absolute(AbsoluteLength::Rems(rems(0.375))))
        );
        assert_eq!(
            padding.style().padding.right,
            Some(DefiniteLength::Absolute(AbsoluteLength::Rems(rems(0.375))))
        );

        let mut margin = div().my_2p5().pt_3p5();
        assert_eq!(
            margin.style().margin.top,
            Some(DefiniteLength::Absolute(AbsoluteLength::Rems(rems(0.625))).into())
        );
        assert_eq!(
            margin.style().padding.top,
            Some(DefiniteLength::Absolute(AbsoluteLength::Rems(rems(0.875))).into())
        );
    }
}

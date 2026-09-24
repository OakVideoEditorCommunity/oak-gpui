//! Panel identity, the [`DockPanel`](crate::dock::DockPanel) trait, and type-erased panel handles.
//!
//! A *panel* is any view the user can dock. It is an ordinary GPUI view
//! ([`Render`]) that additionally implements [`DockPanel`](crate::dock::DockPanel) so the dock system
//! can identify it, label its tab, and negotiate closing. The [`DockArea`](crate::dock::DockArea)
//! stores panels as [`PanelHandle`]s — a type-erased wrapper around
//! [`AnyView`] plus the metadata the dock chrome (tab strip, drop overlay)
//! needs without downcasting.

use crate::{
	AnyElement, AnyView, App, Context, Entity, EventEmitter, Render, SharedString, Subscription,
	Window,
};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Stable, copyable identifier for a docked panel.
///
/// A `PanelId` uniquely identifies one panel instance within a [`DockArea`](crate::dock::DockArea)
/// for its whole lifetime: the layout tree ([`DockNode`](crate::dock::DockNode))
/// refers to panels exclusively by id, and events such as
/// [`DockEvent::PanelFocused`](crate::dock::DockEvent::PanelFocused) carry it.
///
/// Ids are assigned by the panel implementation (or the application) via
/// [`DockPanel::panel_id`]. They must be unique within a dock area; adding a
/// second panel with an existing id is an error (see
/// [`DockArea::add_panel`](crate::dock::DockArea::add_panel)(crate::dock::DockArea::add_panel)). Ids are *not*
/// required to be stable across sessions — persistence goes through string
/// keys, see [`PanelRegistry`](crate::dock::PanelRegistry).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PanelId(u64);

impl PanelId {
	/// Creates a panel id from a raw numeric value.
	///
	/// The value only needs to be unique within the owning
	/// [`DockArea`](crate::dock::DockArea)(crate::dock::DockArea); a simple per-application counter
	/// (or a hash of a stable name) is sufficient.
	pub const fn new(raw: u64) -> Self {
		Self(raw)
	}

	/// Returns the raw numeric value backing this id.
	pub const fn raw(self) -> u64 {
		self.0
	}
}

impl fmt::Display for PanelId {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "panel-{}", self.0)
	}
}

/// Events a dock panel can emit to its containing [`DockArea`](crate::dock::DockArea)(crate::dock::DockArea).
///
/// Panels emit these through their [`EventEmitter<PanelEvent>`] implementation
/// (required by [`DockPanel`](crate::dock::DockPanel)). The dock area subscribes to every panel it
/// holds and reacts — e.g. by updating the tab label or starting the close
/// flow — without the panel needing a direct reference to the dock.
#[derive(Clone, Debug)]
pub enum PanelEvent {
	/// The panel asked to be closed (e.g. its own close affordance was
	/// invoked).
	///
	/// The dock area does not remove the panel unconditionally: it first
	/// consults [`DockPanel::should_close`], then calls [`DockPanel::on_close`]
	/// and removes the panel only if closing was confirmed.
	CloseRequested,
	/// The panel's content gained keyboard focus.
	///
	/// The dock area uses this to keep its own `focused_panel` bookkeeping in
	/// sync and to emit
	/// [`DockEvent::PanelFocused`](crate::dock::DockEvent::PanelFocused).
	Focused,
	/// The panel's title changed; the tab strip should re-render the label.
	///
	/// The new title is read back through [`DockPanel::title`] rather than
	/// carried in the event, so panels never have to clone it.
	TitleChanged,
}

/// A view that can live inside a [`DockArea`](crate::dock::DockArea)(crate::dock::DockArea).
///
/// Implement this on the same view type that implements [`Render`]. The dock
/// area renders the panel's normal [`Render::render`] output as the tab
/// group's content; this trait only supplies dock-specific metadata and
/// lifecycle hooks.
///
/// # Required items
///
/// - [`panel_id`](DockPanel::panel_id) — stable identity.
/// - [`title`](DockPanel::title) — tab label.
/// - [`tab_content`](DockPanel::tab_content) — rich tab content (icon + label,
///   status dot, ...).
///
/// # Provided items
///
/// - [`closable`](DockPanel::closable) — whether a close button is shown
///   (default `true`).
/// - [`should_close`](DockPanel::should_close) — veto hook, e.g. an unsaved
///   changes confirmation (default `true`).
/// - [`on_close`](DockPanel::on_close) — cleanup hook run after a confirmed
///   close.
///
/// # Examples
///
/// ```ignore
/// struct MediaBin { /* ... */ }
///
/// impl Render for MediaBin { /* ... */ }
/// impl EventEmitter<PanelEvent> for MediaBin {}
///
/// impl DockPanel for MediaBin {
///     fn panel_id(&self) -> PanelId { PanelId::new(1) }
///     fn title(&self, _cx: &App) -> SharedString { "Media Bin".into() }
///     fn tab_content(&self, _cx: &App) -> AnyElement {
///         div().child("Media Bin").into_any_element()
///     }
/// }
/// ```
pub trait DockPanel: Render + EventEmitter<PanelEvent> + 'static {
	/// Returns the stable id of this panel.
	///
	/// Must return the same value for the whole lifetime of the view and must
	/// be unique among all panels added to one dock area.
	fn panel_id(&self) -> PanelId;

	/// Returns the plain-text title shown in the tab strip and, where
	/// relevant, in window titles for floated panels.
	///
	/// Called on every render of the containing tab bar, so it should be
	/// cheap. Emit [`PanelEvent::TitleChanged`] after changing whatever state
	/// feeds this.
	fn title(&self, cx: &App) -> SharedString;

	/// Returns the element rendered inside this panel's tab.
	///
	/// The default tab bar renders [`title`](DockPanel::title) when this is
	/// not customized, but panels may return richer content (icon, dirty
	/// indicator, close-on-middle-click affordances). The returned element
	/// must not handle close or drag interactions itself — the tab strip
	/// overlays those.
	fn tab_content(&self, cx: &App) -> AnyElement;

	/// Whether this panel shows a close button and can be closed by the user.
	///
	/// Non-closable panels can still be removed programmatically via
	/// [`DockArea::remove_panel`](crate::dock::DockArea::remove_panel).
	/// Defaults to `true`.
	fn closable(&self) -> bool {
		true
	}

	/// Called when the user has asked to close this panel, before removal.
	///
	/// Return `true` to allow the close, `false` to veto it (for example
	/// after showing an "unsaved changes" dialog). This may be called on the
	/// same event-loop turn as the close request, so asynchronous
	/// confirmations should veto now and re-trigger closing later through
	/// [`DockArea::remove_panel`](crate::dock::DockArea::remove_panel).
	/// Defaults to `true`.
	fn should_close(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> bool {
		true
	}

	/// Called after a close was confirmed and before the panel is removed
	/// from the layout.
	///
	/// Use this to release resources tied to the dock (subscriptions,
	/// scratch entities). The default implementation does nothing.
	fn on_close(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}
}

/// A type-erased panel plus the metadata the dock chrome needs.
///
/// Wraps the panel's view as an [`AnyView`] so a [`DockArea`](crate::dock::DockArea)(crate::dock::DockArea)
/// can hold heterogeneous panel types in one collection. The metadata
/// ([`PanelId`], title, closability) is a cached snapshot taken at
/// construction and refreshed when the panel emits [`PanelEvent::TitleChanged`].
///
/// Construct with [`PanelHandle::new`]; pass to
/// [`DockArea::add_panel`](crate::dock::DockArea::add_panel)(crate::dock::DockArea::add_panel).
pub struct PanelHandle {
	id: PanelId,
	view: AnyView,
	title: SharedString,
	closable: bool,
	/// Re-reads [`DockPanel::title`] into the cached snapshot. The handle is
	/// type-erased at this point, so the refresh goes through a closure
	/// captured from the concrete entity (used when the UI language
	/// switches; the panels' titles are localized).
	title_provider: Box<dyn Fn(&App) -> SharedString>,
	/// Marks the panel view dirty so its localized content re-renders
	/// (same language-switch path as `title_provider`).
	notifier: Box<dyn Fn(&mut App)>,
	/// Subscription to the panel's [`PanelEvent`]s while it is held by a
	/// dock area, installed by [`DockArea`](crate::dock::DockArea) when the
	/// panel is added.
	subscription: Option<Subscription>,
}

impl PanelHandle {
	/// Wraps a panel view, snapshotting its current metadata.
	///
	/// `panel` must implement [`DockPanel`](crate::dock::DockPanel). The handle keeps the view alive
	/// for as long as it is stored in the dock area.
	///
	/// # Panics
	///
	/// Does not panic, but adding two handles with the same
	/// [`DockPanel::panel_id`] to one dock area is rejected there.
	pub fn new<P: DockPanel>(panel: Entity<P>, cx: &App) -> Self {
		let id = panel.read(cx).panel_id();
		let title = panel.read(cx).title(cx);
		let closable = panel.read(cx).closable();
		let title_panel = panel.clone();
		let title_provider = Box::new(move |cx: &App| title_panel.read(cx).title(cx));
		let notify_panel = panel.clone();
		let notifier = Box::new(move |cx: &mut App| {
			notify_panel.update(cx, |_panel, cx| cx.notify());
		});
		Self {
			id,
			view: panel.into(),
			title,
			closable,
			title_provider,
			notifier,
			subscription: None,
		}
	}

	/// Returns the id the panel reported at snapshot time.
	pub fn panel_id(&self) -> PanelId {
		self.id
	}

	/// Returns the cached tab title.
	///
	/// May be stale between a title change and the dock area processing
	/// [`PanelEvent::TitleChanged`]; treat as display-only.
	pub fn title(&self) -> &SharedString {
		&self.title
	}

	/// Re-reads [`DockPanel::title`] from the panel view into the cached
	/// snapshot. Used by [`DockArea::refresh_panel_titles`](crate::dock::DockArea::refresh_panel_titles)
	/// when the UI language switches (the titles are localized).
	pub(crate) fn refresh_title(&mut self, cx: &App) {
		self.title = (self.title_provider)(cx);
	}

	/// Marks the panel view dirty so its localized content re-renders.
	pub(crate) fn notify_panel(&self, cx: &mut App) {
		(self.notifier)(cx);
	}

	/// Returns the cached value of [`DockPanel::closable`].
	pub fn closable(&self) -> bool {
		self.closable
	}

	/// Returns the type-erased panel view.
	pub fn view(&self) -> &AnyView {
		&self.view
	}

	/// Returns the subscription to this panel's [`PanelEvent`]s, if the dock
	/// area has installed one.
	#[allow(dead_code)] // reserved for the dock's panel-event bookkeeping
	pub(crate) fn subscription(&self) -> &Option<Subscription> {
		&self.subscription
	}

	/// Installs (or replaces) the subscription to this panel's [`PanelEvent`]s.
	///
	/// Used by [`DockArea`](crate::dock::DockArea) when the panel is added or
	/// restored; the previous subscription, if any, is dropped.
	pub(crate) fn set_subscription(&mut self, subscription: Option<Subscription>) {
		self.subscription = subscription;
	}
}

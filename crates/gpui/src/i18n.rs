//! Minimal localization hook for widget-baked strings.
//!
//! A few widgets embed small, user-visible strings directly (transport
//! buttons, empty states). Rather than shipping a full i18n framework into
//! the widget crates, they expose a single override point: a process-global
//! string table that maps a stable key to a localized string. Widget code
//! calls [`tr`] with the key plus the string it would otherwise show; when
//! the installed table has an entry for the key that entry wins, otherwise
//! the built-in default is used.
//!
//! This means:
//!
//! * An app that never calls [`set_table`] sees exactly the strings baked
//!   into the widgets (no behavior change, all tests keep passing).
//! * An app that wants localized widgets installs a [`StringTable`] once per
//!   language (e.g. on startup and on every language switch) and every
//!   widget picks the new strings up on the next render.
//!
//! The table is a plain `HashMap<String, String>` — no serde, no build step
//! — behind a single [`RwLock`], so any thread may install or read it. The
//! companion `gpui_widgets::i18n` module re-exports this API so hosts can
//! address it as `gpui_widgets::i18n::set_table(..)`.

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

use crate::SharedString;

/// A key → localized-string mapping installed by the host application.
pub type StringTable = HashMap<String, String>;

/// The installed table, or `None` (built-in defaults) when unset.
fn table() -> &'static RwLock<Option<StringTable>> {
	static TABLE: OnceLock<RwLock<Option<StringTable>>> = OnceLock::new();
	TABLE.get_or_init(|| RwLock::new(None))
}

/// Installs `strings` as the string-table override, replacing any previously
/// installed table wholesale.
pub fn set_table(strings: StringTable) {
	*table().write().unwrap() = Some(strings);
}

/// Removes the override so every string falls back to its built-in default.
pub fn clear_table() {
	*table().write().unwrap() = None;
}

/// Returns the localized string for `key`, or `default` when the installed
/// table has no entry for it.
pub fn tr(key: &str, default: impl Into<SharedString>) -> SharedString {
	if let Some(value) = table()
		.read()
		.unwrap()
		.as_ref()
		.and_then(|strings| strings.get(key))
	{
		SharedString::from(value.clone())
	} else {
		default.into()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn missing_table_uses_defaults() {
		clear_table();
		assert_eq!(tr("viewer.safe_frames", "安全框"), "安全框");
		assert_eq!(tr("viewer.zoom", "缩放"), "缩放");
	}

	#[test]
	fn installed_table_overrides_defaults() {
		let mut table = StringTable::new();
		table.insert("viewer.safe_frames".into(), "Safe Frames".into());
		set_table(table);
		assert_eq!(tr("viewer.safe_frames", "安全框"), "Safe Frames");
		// Keys not in the table keep their defaults.
		assert_eq!(tr("viewer.zoom", "缩放"), "缩放");
		clear_table();
		assert_eq!(tr("viewer.safe_frames", "安全框"), "安全框");
	}
}

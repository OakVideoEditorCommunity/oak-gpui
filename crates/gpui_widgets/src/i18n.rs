//! Localization hook for widget-baked strings.
//!
//! Re-exports the [`gpui::i18n`] string-table override API so hosts can
//! localize the widgets' built-in strings as
//! `gpui_widgets::i18n::set_table(..)`. See [`gpui::i18n`] for the full
//! contract: [`tr`] returns the installed override for a key, or the
//! widget's built-in default when none is set.
//!
//! [`tr`]: gpui::i18n::tr

pub use gpui::i18n::*;

#[cfg(test)]
mod tests {
	use super::*;

	/// The hook is a thin re-export: installing a table through the widget
	/// path is visible to `gpui::i18n::tr` and vice versa.
	#[test]
	fn widget_path_shares_the_gpui_table() {
		clear_table();
		assert_eq!(tr("viewer.safe_frames", "安全框"), "安全框");

		let mut table = StringTable::new();
		table.insert("viewer.safe_frames".into(), "Safe Frames".into());
		set_table(table);

		assert_eq!(
			gpui::i18n::tr("viewer.safe_frames", "安全框"),
			"Safe Frames"
		);
		clear_table();
	}
}

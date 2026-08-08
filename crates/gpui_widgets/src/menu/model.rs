//! Pure menu data model: items with nesting, enable/check state, keyboard
//! navigation arithmetic. No gpui coupling, unit-tested.

use gpui::SharedString;

/// A menu item.
#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    /// The item's stable id (used in [`MenuEvent`](super::MenuEvent)).
    pub id: usize,
    /// The label shown in the menu.
    pub label: SharedString,
    /// A shortcut to display on the right (e.g. `"⌘S"`).
    pub shortcut: Option<SharedString>,
    /// Whether the item can be activated.
    pub enabled: bool,
    /// `None` = no checkmark; `Some(checked)` = a check/tick state.
    pub checked: Option<bool>,
    /// A nested submenu, opened on hover/click.
    pub submenu: Option<Box<Menu>>,
    /// Whether a separator line follows this item.
    pub separator_after: bool,
}

impl MenuItem {
    /// Create a plain enabled item.
    pub fn new(id: usize, label: impl Into<SharedString>) -> Self {
        Self {
            id,
            label: label.into(),
            shortcut: None,
            enabled: true,
            checked: None,
            submenu: None,
            separator_after: false,
        }
    }

    /// Mark the item disabled.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Attach a shortcut label.
    pub fn with_shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    /// Set a checked state.
    pub fn with_checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// Attach a submenu.
    pub fn with_submenu(mut self, submenu: Menu) -> Self {
        self.submenu = Some(Box::new(submenu));
        self
    }

    /// Draw a separator line after this item.
    pub fn separated(mut self) -> Self {
        self.separator_after = true;
        self
    }
}

/// A menu: an ordered list of items.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Menu {
    /// The items of this menu.
    pub items: Vec<MenuItem>,
}

impl Menu {
    /// Create a menu.
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self { items }
    }

    /// Whether the item at `index` is a visual separator (an empty item).
    pub fn is_separator(item: &MenuItem) -> bool {
        item.label.is_empty()
    }

    /// The next selectable index from `current`, moving `delta` steps
    /// (skipping separators and disabled items). `None` returns the first
    /// (or last) selectable item. Returns `None` if nothing is selectable.
    pub fn navigate(&self, current: Option<usize>, delta: i32) -> Option<usize> {
        let selectable: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| item.enabled && !Self::is_separator(item))
            .map(|(index, _)| index)
            .collect();
        if selectable.is_empty() {
            return None;
        }
        let position = current.and_then(|c| selectable.iter().position(|&i| i == c));
        let next = match position {
            Some(pos) => {
                ((pos as i64 + delta as i64).rem_euclid(selectable.len() as i64)) as usize
            }
            None if delta > 0 => 0,
            None => selectable.len() - 1,
        };
        Some(selectable[next])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_menu() -> Menu {
        Menu::new(vec![
            MenuItem::new(1, "Open…").with_shortcut("⌘O"),
            MenuItem::new(2, "Save").with_shortcut("⌘S"),
            MenuItem::new(3, "Save As…").with_shortcut("⇧⌘S").separated(),
            MenuItem::new(4, "Export").disabled(),
            MenuItem::new(5, "Export Again").with_checked(true),
        ])
    }

    #[test]
    fn navigate_skips_disabled_and_separators() {
        let menu = sample_menu();
        // From none, down lands on the first selectable (Open).
        assert_eq!(menu.navigate(None, 1), Some(0));
        // From none, up lands on the last selectable (Export Again, idx 4).
        assert_eq!(menu.navigate(None, -1), Some(4));
        // From Open (0), down skips nothing until Save (1).
        assert_eq!(menu.navigate(Some(0), 1), Some(1));
        // From Save As (2), down skips the disabled Export (3) to Export Again (4).
        assert_eq!(menu.navigate(Some(2), 1), Some(4));
        // Wrap around.
        assert_eq!(menu.navigate(Some(4), 1), Some(0));
        assert_eq!(menu.navigate(Some(0), -1), Some(4));
    }

    #[test]
    fn navigate_returns_none_when_nothing_selectable() {
        let menu = Menu::new(vec![
            MenuItem::new(1, "Only").disabled(),
            MenuItem::new(2, ""),
        ]);
        assert_eq!(menu.navigate(None, 1), None);
        assert_eq!(menu.navigate(Some(1), 1), None);
    }

    #[test]
    fn checked_state_is_optional() {
        let menu = sample_menu();
        assert_eq!(menu.items[0].checked, None);
        assert_eq!(menu.items[4].checked, Some(true));
    }

    #[test]
    fn cascade_nesting() {
        let sub = Menu::new(vec![MenuItem::new(10, "A"), MenuItem::new(11, "B")]);
        let item = MenuItem::new(5, "Nested").with_submenu(sub.clone());
        assert_eq!(item.submenu.as_ref().unwrap().items.len(), 2);
        assert_eq!(item.submenu.as_ref().unwrap().items[1].id, 11);
        // The nested menu itself navigates independently.
        assert_eq!(item.submenu.as_ref().unwrap().navigate(None, -1), Some(1));
    }

    #[test]
    fn separator_detection() {
        assert!(Menu::is_separator(&MenuItem::new(0, "")));
        assert!(!Menu::is_separator(&MenuItem::new(0, "Save")));
    }
}

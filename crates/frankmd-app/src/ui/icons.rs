//! Chrome icons from FrankMD's Phosphor set (served by `assets.rs` as
//! `icons/ph/<name>.svg`), so toolbar buttons share one drawn style instead
//! of mixing Unicode glyphs from whatever font has them.

use gpui_kit::component::Icon;

/// The Phosphor icon `name` (e.g. `"gear"`, `"magnifying-glass"`).
pub(crate) fn ph(name: &str) -> Icon {
    Icon::default().path(format!("icons/ph/{name}.svg"))
}

#[cfg(test)]
mod tests {
    /// Every name used in the UI must exist in the bundled set, or the
    /// button renders empty.
    #[test]
    fn chrome_icons_exist() {
        for name in [
            "plus",
            "minus",
            "x",
            "check",
            "folder-plus",
            "file-plus",
            "images",
            "gear",
            "question",
            "image",
            "magnifying-glass",
            "sidebar",
            "dots-three",
            "link",
            "arrow-up",
            "arrow-down",
            "lock",
            "lock-open",
        ] {
            assert!(frankmd_core::emoji::icons::lookup(name).is_some(), "{name}");
        }
    }
}

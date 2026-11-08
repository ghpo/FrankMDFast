//! UI strings: FrankMD's Rails locale files (unchanged, so `/translations`
//! parity and the frankmd-i18n goldens hold) deep-merged with the strings the
//! native UI adds (`assets/locales/native/<code>.yml`: banners, palette,
//! status, settings sheet, help, preview). One [`Strings`] global; switching
//! the language swaps it and refreshes every window (no reload, FE-501).

use std::sync::Arc;

use frankmd_i18n::{Catalog, Locale, Node, deep_merge_over, locale_tree, parse_locale_yaml};
use gpui_kit::{App, Global, SharedString};

/// The native-UI string files, embedded.
fn native_yaml(locale: Locale) -> &'static str {
    match locale {
        Locale::En => include_str!("../../../../assets/locales/native/en.yml"),
        Locale::PtBr => include_str!("../../../../assets/locales/native/pt-BR.yml"),
        Locale::PtPt => include_str!("../../../../assets/locales/native/pt-PT.yml"),
        Locale::Es => include_str!("../../../../assets/locales/native/es.yml"),
        Locale::He => include_str!("../../../../assets/locales/native/he.yml"),
        Locale::Ja => include_str!("../../../../assets/locales/native/ja.yml"),
        Locale::Ko => include_str!("../../../../assets/locales/native/ko.yml"),
    }
}

/// The native-UI tree of `locale` (empty when it fails to parse; covered by tests).
pub fn native_tree(locale: Locale) -> Node {
    parse_locale_yaml(native_yaml(locale), locale.code()).unwrap_or_else(|err| {
        tracing::error!(%err, locale = locale.code(), "native locale file does not parse");
        Node::Map(Vec::new())
    })
}

/// Full tree of `locale`: the Rails file with the native strings over it.
fn full_tree(locale: Locale) -> Node {
    let base = locale_tree(locale).unwrap_or_else(|err| {
        tracing::error!(%err, locale = locale.code(), "locale file does not parse");
        Node::Map(Vec::new())
    });
    deep_merge_over(&base, &native_tree(locale))
}

/// Build the catalog of `locale` (active strings, then English).
pub fn catalog(locale: Locale) -> Arc<Catalog> {
    let english = full_tree(Locale::En);
    let active = if locale == Locale::En {
        english.clone()
    } else {
        full_tree(locale)
    };
    Arc::new(Catalog::from_trees(locale, &active, &english))
}

/// The active catalog.
pub struct Strings(pub Arc<Catalog>);
impl Global for Strings {}

/// Install `locale` as the UI language.
pub fn set_locale(locale: Locale, cx: &mut App) {
    if cx
        .try_global::<Strings>()
        .is_some_and(|s| s.0.locale() == locale)
    {
        return;
    }
    cx.set_global(Strings(catalog(locale)));
    cx.refresh_windows();
}

/// The active catalog (English before [`set_locale`] ran).
pub fn current(cx: &App) -> Arc<Catalog> {
    cx.try_global::<Strings>()
        .map_or_else(|| catalog(Locale::En), |s| Arc::clone(&s.0))
}

/// Translate `key`.
pub fn t(cx: &App, key: &str) -> SharedString {
    t_args(cx, key, &[])
}

/// Translate `key` with `%{var}` values.
pub fn t_args(cx: &App, key: &str, args: &[(&str, &str)]) -> SharedString {
    match cx.try_global::<Strings>() {
        Some(strings) => strings.0.t(key, args).into(),
        None => catalog(Locale::En).t(key, args).into(),
    }
}

/// Translate with pluralization on `count`.
pub fn t_count(cx: &App, key: &str, count: i64) -> SharedString {
    match cx.try_global::<Strings>() {
        Some(strings) => strings.0.t_count(key, count, &[]).into(),
        None => catalog(Locale::En).t_count(key, count, &[]).into(),
    }
}

/// Native name of a locale (`languages.*`).
pub fn language_name(cx: &App, locale: Locale) -> SharedString {
    t(
        cx,
        &format!("languages.{}", locale.code().replace('-', "_")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use frankmd_i18n::translation_failures;

    // Same rule as frankmd-i18n's translation_coverage_test, for the native strings.
    #[test]
    fn native_strings_are_complete_in_every_locale() {
        let english = native_tree(Locale::En);
        assert!(!frankmd_i18n::flatten_leaves(&english).is_empty());
        let others: Vec<(&str, Node)> = Locale::ALL
            .iter()
            .filter(|l| **l != Locale::En)
            .map(|l| (l.code(), native_tree(*l)))
            .collect();
        let refs: Vec<(&str, &Node)> = others.iter().map(|(c, n)| (*c, n)).collect();
        assert_eq!(translation_failures(&english, &refs), Vec::<String>::new());
    }

    #[test]
    fn native_strings_parse_for_every_locale() {
        for locale in Locale::ALL {
            assert!(
                parse_locale_yaml(native_yaml(locale), locale.code()).is_ok(),
                "{}",
                locale.code()
            );
        }
    }

    #[test]
    fn interpolation_variables_match_english() {
        let en = frankmd_i18n::interpolation_vars(&native_tree(Locale::En));
        for locale in Locale::ALL {
            let vars = frankmd_i18n::interpolation_vars(&native_tree(locale));
            assert_eq!(vars, en, "{}", locale.code());
        }
    }

    #[test]
    fn merged_catalog_resolves_native_and_rails_keys() {
        let cat = catalog(Locale::PtBr);
        assert_eq!(cat.t("common.cancel", &[]), "Cancelar");
        assert_ne!(
            cat.t("banners.content_loss.undo", &[]),
            "banners.content_loss.undo"
        );
        assert_ne!(cat.t("status.saving", &[]), "status.saving");
        assert_eq!(cat.t("context_menu.encrypt", &[]), "Criptografar nota");
        assert_eq!(cat.t("context_menu.decrypt", &[]), "Descriptografar nota");
    }
}

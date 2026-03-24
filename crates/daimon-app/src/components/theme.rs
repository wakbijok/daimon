use leptos::prelude::*;

/// Reactive theme signal — "dark" or "light"
#[derive(Clone, Copy)]
pub struct ThemeSignal(pub RwSignal<String>);

/// Initialize theme from user preference or system default.
/// Call once in the root Layout component.
pub fn provide_theme() {
    let theme = RwSignal::new("dark".to_string());
    provide_context(ThemeSignal(theme));

    #[cfg(feature = "hydrate")]
    {
        if let Some(window) = web_sys::window() {
            // Check localStorage for saved preference
            if let Ok(Some(storage)) = window.local_storage() {
                if let Ok(Some(saved)) = storage.get_item("daimon_theme") {
                    theme.set(saved.clone());
                    apply_theme_class(&saved);
                    return;
                }
            }
            // Fall back to system preference
            if let Ok(Some(mq)) = window.match_media("(prefers-color-scheme: light)") {
                if mq.matches() {
                    theme.set("light".to_string());
                    apply_theme_class("light");
                    return;
                }
            }
        }
        apply_theme_class("dark");
    }
}

/// Toggle between dark and light, persist to localStorage
pub fn toggle_theme() {
    if let Some(ThemeSignal(theme)) = use_context::<ThemeSignal>() {
        let new_theme = if theme.get_untracked() == "dark" { "light" } else { "dark" };
        theme.set(new_theme.to_string());

        #[cfg(feature = "hydrate")]
        {
            apply_theme_class(new_theme);
            if let Some(window) = web_sys::window() {
                if let Ok(Some(storage)) = window.local_storage() {
                    let _ = storage.set_item("daimon_theme", new_theme);
                }
            }
        }
    }
}

#[cfg(feature = "hydrate")]
fn apply_theme_class(theme: &str) {
    if let Some(window) = web_sys::window() {
        if let Some(doc) = window.document() {
            if let Some(el) = doc.document_element() {
                let _ = el.class_list().remove_1("dark");
                let _ = el.class_list().remove_1("light");
                let _ = el.class_list().add_1(theme);
            }
        }
    }
}

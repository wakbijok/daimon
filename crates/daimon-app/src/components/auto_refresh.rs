//! Auto-refresh component — polling-based with interval selector.
//!
//! Provides a `RefreshSignal` context that other components can use as a
//! Resource dependency to trigger re-fetches on a timer. The `RefreshSelector`
//! component renders an interval dropdown (5s/15s/30s/60s/Off) and manages
//! the polling interval via `gloo_timers::callback::Interval` on the client.

use leptos::prelude::*;

/// Incrementing counter that triggers Resource refresh when it changes.
#[derive(Clone, Copy)]
pub struct RefreshSignal(pub RwSignal<u64>);

/// Call once in Layout to provide the refresh signal context.
pub fn provide_refresh() {
    let counter = RwSignal::new(0u64);
    provide_context(RefreshSignal(counter));
}

/// Get the refresh counter — use as a Resource dependency to trigger re-fetch.
///
/// Example:
/// ```ignore
/// let refresh = use_refresh_counter();
/// let data = Resource::new(refresh, |_| async { fetch_data().await });
/// ```
pub fn use_refresh_counter() -> impl Fn() -> u64 + Clone + Copy {
    let RefreshSignal(counter) = expect_context::<RefreshSignal>();
    move || counter.get()
}

/// Interval selector dropdown with auto-refresh status indicator.
#[component]
pub fn RefreshSelector() -> impl IntoView {
    #[allow(unused_variables)]
    let RefreshSignal(counter) = expect_context::<RefreshSignal>();
    let (interval_secs, set_interval_secs) = signal(30u64);
    let (paused, set_paused) = signal(false);

    // Start polling interval on hydrate (client-side only)
    #[cfg(feature = "hydrate")]
    {
        use gloo_timers::callback::Interval;
        let handle = StoredValue::new(None::<Interval>);

        Effect::new(move |_| {
            let secs = interval_secs.get();
            let is_paused = paused.get();

            // Clear old interval
            handle.set_value(None);

            if !is_paused && secs > 0 {
                let interval = Interval::new(secs as u32 * 1000, move || {
                    counter.update(|c| *c += 1);
                });
                handle.set_value(Some(interval));
            }
        });
    }

    view! {
        <div class="flex items-center gap-2">
            <select
                on:change=move |ev| {
                    let val: u64 = event_target_value(&ev).parse().unwrap_or(30);
                    if val == 0 {
                        set_paused.set(true);
                    } else {
                        set_paused.set(false);
                        set_interval_secs.set(val);
                    }
                }
                class="px-2 py-1 text-xs bg-surface-secondary border border-border-primary rounded-md text-text-muted"
            >
                <option value="5">"5s"</option>
                <option value="15">"15s"</option>
                <option value="30" selected=true>"30s"</option>
                <option value="60">"60s"</option>
                <option value="0">"Off"</option>
            </select>
            <span class="text-[10px] text-text-muted">
                {move || if paused.get() { "Paused".to_string() } else { format!("Auto-refresh: {}s", interval_secs.get()) }}
            </span>
        </div>
    }
}

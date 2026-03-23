use leptos::prelude::*;
use leptos_router::components::Outlet;
use super::sidebar::Sidebar;
use crate::auth_guard::get_current_user;

#[component]
pub fn Layout() -> impl IntoView {
    let user = Resource::new(|| (), |_| get_current_user());

    view! {
        <Suspense fallback=|| view! { <div class="min-h-screen bg-surface-primary" /> }>
            {move || user.get().map(|result| match result {
                Ok(Some(_username)) => view! {
                    <div class="flex min-h-screen bg-surface-primary text-text-primary">
                        <Sidebar />
                        <main class="flex-1 min-w-0 p-4 sm:p-6">
                            <div class="max-w-[1400px] mx-auto">
                                <Outlet />
                            </div>
                        </main>
                    </div>
                }.into_any(),
                _ => view! {
                    <RedirectToLogin />
                }.into_any(),
            })}
        </Suspense>
    }
}

#[component]
fn RedirectToLogin() -> impl IntoView {
    #[cfg(feature = "hydrate")]
    {
        if let Some(window) = web_sys::window() {
            let _ = window.location().set_href("/login");
        }
    }
    view! { <div class="min-h-screen bg-surface-primary" /> }
}

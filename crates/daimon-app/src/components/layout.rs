use leptos::prelude::*;
use leptos_router::components::Outlet;
use super::sidebar::Sidebar;

#[component]
pub fn Layout() -> impl IntoView {
    view! {
        <div class="flex min-h-screen bg-surface-primary text-text-primary">
            <Sidebar />
            <main class="flex-1 min-w-0 p-4 sm:p-6">
                <div class="max-w-[1400px] mx-auto">
                    <Outlet />
                </div>
            </main>
        </div>
    }
}

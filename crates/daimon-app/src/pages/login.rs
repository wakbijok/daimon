use leptos::prelude::*;

#[component]
pub fn Login() -> impl IntoView {
    view! {
        <div class="min-h-screen flex items-center justify-center bg-surface-primary">
            <div class="w-full max-w-sm p-8 bg-surface-secondary rounded-lg border border-border-primary">
                <h1 class="text-xl font-bold text-center mb-6">
                    <span class="text-text-primary">"dai"</span>
                    <span class="text-accent-amber">"mon"</span>
                </h1>
                <form class="space-y-4">
                    <div>
                        <label class="block text-sm text-text-secondary mb-1">"Username"</label>
                        <input
                            type="text"
                            class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            placeholder="admin"
                        />
                    </div>
                    <div>
                        <label class="block text-sm text-text-secondary mb-1">"Password"</label>
                        <input
                            type="password"
                            class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        />
                    </div>
                    <button
                        type="submit"
                        class="w-full py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm"
                    >
                        "Sign in"
                    </button>
                </form>
            </div>
        </div>
    }
}

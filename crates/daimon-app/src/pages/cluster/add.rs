use leptos::prelude::*;

#[server]
async fn test_connection(api_url: String, token: String) -> Result<String, ServerFnError> {
    let client = daimon_pve::Client::from_token_string(&api_url, &token);
    let version = client.version().await.map_err(|e| ServerFnError::new(e.to_string()))?;
    Ok(format!("PVE {} ({})", version.version, version.release))
}

#[server]
async fn save_cluster(name: String, api_url: String, token: String, notes: String) -> Result<String, ServerFnError> {
    use crate::state::AppState;
    use crate::db;

    // Test connection first
    let client = daimon_pve::Client::from_token_string(&api_url, &token);
    client.version().await.map_err(|e| ServerFnError::new(format!("Connection failed: {}", e)))?;

    let state = expect_context::<AppState>();
    let id = uuid::Uuid::new_v4().to_string();

    {
        let conn = state.db.lock().await;
        db::insert_cluster(&conn, &id, &name, &api_url, &token, &notes)
            .map_err(|e| ServerFnError::new(e.to_string()))?;
    }

    // Add to in-memory PVE clients
    {
        let new_client = daimon_pve::Client::from_token_string(&api_url, &token);
        let mut clients = state.pve_clients.write().await;
        clients.insert(id.clone(), new_client);
    }

    Ok(id)
}

#[component]
pub fn AddCluster() -> impl IntoView {
    let (name, set_name) = signal(String::new());
    let (api_url, set_api_url) = signal(String::new());
    let (token, set_token) = signal(String::new());
    let (notes, set_notes) = signal(String::new());
    let (test_result, set_test_result) = signal(Option::<Result<String, String>>::None);
    let (saving, set_saving) = signal(false);

    let test_action = Action::new(move |_: &()| {
        let url = api_url.get_untracked();
        let tok = token.get_untracked();
        async move {
            let result = test_connection(url, tok).await;
            set_test_result.set(Some(result.map_err(|e| e.to_string())));
        }
    });

    let save_action = Action::new(move |_: &()| {
        let n = name.get_untracked();
        let u = api_url.get_untracked();
        let t = token.get_untracked();
        let no = notes.get_untracked();
        async move {
            set_saving.set(true);
            let result = save_cluster(n, u, t, no).await;
            set_saving.set(false);
            match result {
                Ok(_id) => {
                    #[cfg(feature = "hydrate")]
                    {
                        if let Some(window) = web_sys::window() {
                            let _ = window.location().set_href(&format!("/clusters/{}/nodes", _id));
                        }
                    }
                }
                Err(e) => {
                    set_test_result.set(Some(Err(e.to_string())));
                }
            }
        }
    });

    view! {
        <div class="max-w-lg">
            <h1 class="text-xl font-semibold text-text-primary mb-6">"Add PVE Cluster"</h1>

            <div class="space-y-4">
                <div>
                    <label class="block text-sm text-text-secondary mb-1">"Cluster Name"</label>
                    <input
                        type="text"
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        placeholder="My PVE Cluster"
                        prop:value=move || name.get()
                        on:input=move |ev| set_name.set(event_target_value(&ev))
                    />
                </div>

                <div>
                    <label class="block text-sm text-text-secondary mb-1">"API URL"</label>
                    <input
                        type="text"
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        placeholder="https://pve.example.com:8006"
                        prop:value=move || api_url.get()
                        on:input=move |ev| set_api_url.set(event_target_value(&ev))
                    />
                </div>

                <div>
                    <label class="block text-sm text-text-secondary mb-1">"API Token"</label>
                    <input
                        type="password"
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        placeholder="user@realm!tokenname=token-value"
                        prop:value=move || token.get()
                        on:input=move |ev| set_token.set(event_target_value(&ev))
                    />
                </div>

                <div>
                    <label class="block text-sm text-text-secondary mb-1">"Notes (optional)"</label>
                    <input
                        type="text"
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        prop:value=move || notes.get()
                        on:input=move |ev| set_notes.set(event_target_value(&ev))
                    />
                </div>

                // Test result
                <Show when=move || test_result.get().is_some()>
                    {move || test_result.get().map(|r| match r {
                        Ok(msg) => view! {
                            <div class="p-2 bg-green-500/10 border border-green-500/30 rounded text-green-400 text-sm">{msg}</div>
                        }.into_any(),
                        Err(msg) => view! {
                            <div class="p-2 bg-accent-danger/10 border border-accent-danger/30 rounded text-accent-danger text-sm">{msg}</div>
                        }.into_any(),
                    })}
                </Show>

                <div class="flex gap-3">
                    <button
                        on:click=move |_| { test_action.dispatch(()); }
                        class="px-4 py-2 bg-surface-tertiary border border-border-primary text-text-secondary rounded-md hover:text-text-primary hover:bg-surface-tertiary/80 transition-colors text-sm"
                    >
                        "Test Connection"
                    </button>
                    <button
                        on:click=move |_| { save_action.dispatch(()); }
                        class="px-4 py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm"
                        disabled=move || saving.get()
                    >
                        {move || if saving.get() { "Saving..." } else { "Save Cluster" }}
                    </button>
                </div>
            </div>
        </div>
    }
}

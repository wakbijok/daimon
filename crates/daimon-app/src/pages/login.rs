use leptos::prelude::*;

#[server]
async fn login_action(username: String, password: String) -> Result<bool, ServerFnError> {
    use crate::auth;
    use crate::db;
    use crate::state::AppState;
    use axum::http::header::SET_COOKIE;
    use std::time::{SystemTime, UNIX_EPOCH};

    let state = expect_context::<AppState>();
    let conn = state.db.lock().unwrap();

    let (user_id, _username, hash) = db::find_user(&conn, &username)
        .ok_or_else(|| ServerFnError::new("Invalid credentials"))?;

    if !auth::verify_password(&password, &hash) {
        return Err(ServerFnError::new("Invalid credentials"));
    }

    let session_id = auth::generate_secret();
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let expires_at = (now + 86400).to_string();

    db::insert_session(&conn, &session_id, user_id, &expires_at).unwrap();

    let token = auth::create_jwt(&state.jwt_secret, &username, user_id, &session_id);

    let cookie = format!(
        "daimon_token={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=86400",
        token
    );
    let response_options = expect_context::<leptos_axum::ResponseOptions>();
    response_options.insert_header(SET_COOKIE, cookie.parse().unwrap());

    Ok(true)
}

#[component]
pub fn Login() -> impl IntoView {
    let login = ServerAction::<LoginAction>::new();
    let (error, set_error) = signal(Option::<String>::None);

    Effect::new(move || {
        if let Some(Ok(true)) = login.value().get() {
            // Redirect via full page reload to ensure cookie is sent
            #[cfg(feature = "hydrate")]
            {
                if let Some(window) = web_sys::window() {
                    let _ = window.location().set_href("/");
                }
            }
        }
        if let Some(Err(e)) = login.value().get() {
            set_error.set(Some(e.to_string()));
        }
    });

    view! {
        <div class="min-h-screen flex items-center justify-center bg-surface-primary">
            <div class="w-full max-w-sm p-8 bg-surface-secondary rounded-lg border border-border-primary">
                <h1 class="text-xl font-bold text-center mb-6">
                    <span class="text-text-primary">"dai"</span>
                    <span class="text-accent-amber">"mon"</span>
                </h1>

                <Show when=move || error.get().is_some()>
                    <div class="mb-4 p-2 bg-accent-danger/10 border border-accent-danger/30 rounded text-accent-danger text-sm text-center">
                        {move || error.get().unwrap_or_default()}
                    </div>
                </Show>

                <ActionForm action=login attr:class="space-y-4">
                    <div>
                        <label class="block text-sm text-text-secondary mb-1">"Username"</label>
                        <input
                            type="text"
                            name="username"
                            class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            placeholder="admin"
                            required
                        />
                    </div>
                    <div>
                        <label class="block text-sm text-text-secondary mb-1">"Password"</label>
                        <input
                            type="password"
                            name="password"
                            class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                            required
                        />
                    </div>
                    <button
                        type="submit"
                        class="w-full py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm"
                        disabled=move || login.pending().get()
                    >
                        {move || if login.pending().get() { "Signing in..." } else { "Sign in" }}
                    </button>
                </ActionForm>
            </div>
        </div>
    }
}

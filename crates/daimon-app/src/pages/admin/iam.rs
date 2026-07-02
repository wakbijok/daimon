//! `/admin/iam` — admin-gated user management UI (P1 FR-IAM-11/12).
//!
//! Minimal surface: a user table (username / status / roles / last login) with
//! per-row Enable/Disable and per-role toggle chips, plus a create-user form.
//! All actions dispatch the `crate::iam::*` server-fns, which re-check
//! `require_admin()` server-side (client-side is UX, not authorization).

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::iam::{
    assign_role, create_user, list_users, revoke_role, set_user_status, UserSummary,
};

/// The single-org role catalogue offered in the UI (matches migration V025).
const ROLE_CATALOGUE: &[&str] = &["admin", "operator", "approver", "read-only", "auditor"];

#[component]
pub fn AdminIam() -> impl IntoView {
    let refresh = RwSignal::new(0u64);
    let users_res = Resource::new(move || refresh.get(), |_| async move { list_users().await });
    let create_open = RwSignal::new(false);

    view! {
        <div>
            <div class="flex items-center justify-between mb-6">
                <h1 class="text-xl font-semibold text-text-primary">"Users & Roles"</h1>
                <button
                    type="button"
                    on:click=move |_| create_open.set(true)
                    class="px-4 py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm"
                >
                    "Add User"
                </button>
            </div>

            <Suspense fallback=|| view! {
                <p class="text-text-muted text-sm">"Loading users..."</p>
            }>
                {move || users_res.get().map(|result| match result {
                    Ok(users) if users.is_empty() => view! {
                        <p class="text-text-muted text-sm">"No users."</p>
                    }.into_any(),
                    Ok(users) => view! {
                        <table class="w-full text-sm">
                            <thead>
                                <tr class="text-text-secondary text-left border-b border-border-primary">
                                    <th class="py-2 font-medium">"Username"</th>
                                    <th class="py-2 font-medium">"Status"</th>
                                    <th class="py-2 font-medium">"Roles"</th>
                                    <th class="py-2 font-medium">"Last login"</th>
                                    <th class="py-2 font-medium">"Actions"</th>
                                </tr>
                            </thead>
                            <tbody>
                                {users.into_iter().map(|u| view! {
                                    <UserRowView user=u refresh=refresh />
                                }).collect_view()}
                            </tbody>
                        </table>
                    }.into_any(),
                    Err(e) => view! {
                        <p class="text-accent-danger text-sm">{e.to_string()}</p>
                    }.into_any(),
                })}
            </Suspense>

            <CreateUserModal open=create_open refresh=refresh />
        </div>
    }
}

#[component]
fn UserRowView(user: UserSummary, refresh: RwSignal<u64>) -> impl IntoView {
    let user_id = user.id;
    let status = user.status.clone();
    let is_active = status == "active";
    let roles = user.roles.clone();
    let last_login = user.last_login_at.clone().unwrap_or_else(|| "—".into());

    let toggle_status = move |_| {
        let next = if is_active { "disabled" } else { "active" };
        spawn_local(async move {
            let _ = set_user_status(user_id, next.to_string()).await;
            refresh.update(|n| *n += 1);
        });
    };

    let status_class = if is_active {
        "inline-flex items-center px-2 py-0.5 rounded-full text-[11px] font-medium bg-accent-amber/10 text-accent-amber border border-accent-amber/30"
    } else {
        "inline-flex items-center px-2 py-0.5 rounded-full text-[11px] font-medium bg-surface-tertiary text-text-muted border border-border-primary"
    };
    let status_label = status.clone();

    view! {
        <tr class="border-b border-border-primary/50">
            <td class="py-2 text-text-primary font-medium">{user.username.clone()}</td>
            <td class="py-2">
                <span class=status_class>{status_label}</span>
            </td>
            <td class="py-2">
                <div class="flex flex-wrap gap-1">
                    {ROLE_CATALOGUE.iter().map(|slug| {
                        let slug = slug.to_string();
                        let held = roles.iter().any(|r| r == &slug);
                        let slug_for_click = slug.clone();
                        let chip_class = if held {
                            "px-2 py-0.5 rounded-full text-[11px] font-medium bg-accent-amber/15 text-accent-amber border border-accent-amber/40 cursor-pointer"
                        } else {
                            "px-2 py-0.5 rounded-full text-[11px] font-medium bg-surface-tertiary text-text-muted border border-border-primary hover:text-text-secondary cursor-pointer"
                        };
                        view! {
                            <button
                                type="button"
                                class=chip_class
                                on:click=move |_| {
                                    let slug = slug_for_click.clone();
                                    spawn_local(async move {
                                        let _ = if held {
                                            revoke_role(user_id, slug).await
                                        } else {
                                            assign_role(user_id, slug).await
                                        };
                                        refresh.update(|n| *n += 1);
                                    });
                                }
                            >
                                {slug}
                            </button>
                        }
                    }).collect_view()}
                </div>
            </td>
            <td class="py-2 text-text-muted text-[12px]">{last_login}</td>
            <td class="py-2">
                <button
                    type="button"
                    on:click=toggle_status
                    class=if is_active {
                        "px-2 py-1 text-[12px] rounded text-accent-danger hover:bg-accent-danger/10 transition-colors"
                    } else {
                        "px-2 py-1 text-[12px] rounded text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition-colors"
                    }
                >
                    {if is_active { "Disable" } else { "Enable" }}
                </button>
            </td>
        </tr>
    }
}

#[component]
fn CreateUserModal(open: RwSignal<bool>, refresh: RwSignal<u64>) -> impl IntoView {
    use crate::components::modal::Modal;

    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let selected_roles = RwSignal::new(vec!["read-only".to_string()]);
    let error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);

    Effect::new(move |prev: Option<bool>| {
        let is_open = open.get();
        let was_open = prev.unwrap_or(false);
        if is_open && !was_open {
            username.set(String::new());
            password.set(String::new());
            selected_roles.set(vec!["read-only".to_string()]);
            error.set(None);
            saving.set(false);
        }
        is_open
    });

    let save = move |_| {
        let u = username.get_untracked();
        let p = password.get_untracked();
        let roles = selected_roles.get_untracked();
        if u.trim().is_empty() {
            error.set(Some("Username is required".into()));
            return;
        }
        if p.is_empty() {
            error.set(Some("Password is required".into()));
            return;
        }
        error.set(None);
        saving.set(true);
        spawn_local(async move {
            let result = create_user(u, p, roles).await;
            saving.set(false);
            match result {
                Ok(_) => {
                    open.set(false);
                    refresh.update(|n| *n += 1);
                }
                Err(e) => error.set(Some(format!("{e}"))),
            }
        });
    };

    view! {
        <Modal title="Add User".to_string() open=open>
            <div class="space-y-4">
                <div>
                    <label class="block text-sm text-text-secondary mb-1">"Username"</label>
                    <input
                        type="text"
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        prop:value=move || username.get()
                        on:input=move |ev| username.set(event_target_value(&ev))
                    />
                </div>
                <div>
                    <label class="block text-sm text-text-secondary mb-1">"Password"</label>
                    <input
                        type="password"
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        prop:value=move || password.get()
                        on:input=move |ev| password.set(event_target_value(&ev))
                    />
                </div>
                <div>
                    <label class="block text-sm text-text-secondary mb-1">"Roles"</label>
                    <div class="flex flex-wrap gap-1">
                        {ROLE_CATALOGUE.iter().map(|slug| {
                            let slug = slug.to_string();
                            let slug_for_click = slug.clone();
                            let slug_for_class = slug.clone();
                            view! {
                                <button
                                    type="button"
                                    class=move || {
                                        let held = selected_roles.get().iter().any(|r| r == &slug_for_class);
                                        if held {
                                            "px-2 py-0.5 rounded-full text-[11px] font-medium bg-accent-amber/15 text-accent-amber border border-accent-amber/40 cursor-pointer"
                                        } else {
                                            "px-2 py-0.5 rounded-full text-[11px] font-medium bg-surface-tertiary text-text-muted border border-border-primary hover:text-text-secondary cursor-pointer"
                                        }
                                    }
                                    on:click=move |_| {
                                        let slug = slug_for_click.clone();
                                        selected_roles.update(|rs| {
                                            if let Some(pos) = rs.iter().position(|r| r == &slug) {
                                                rs.remove(pos);
                                            } else {
                                                rs.push(slug);
                                            }
                                        });
                                    }
                                >
                                    {slug}
                                </button>
                            }
                        }).collect_view()}
                    </div>
                </div>
                {move || error.get().map(|e| view! {
                    <div class="p-2 bg-accent-danger/10 border border-accent-danger/30 rounded text-accent-danger text-sm">{e}</div>
                })}
                <div class="flex gap-3 justify-end pt-2">
                    <button
                        type="button"
                        on:click=move |_| open.set(false)
                        class="px-4 py-2 bg-surface-tertiary border border-border-primary rounded-md hover:bg-surface-tertiary/80 transition-colors text-sm text-text-secondary"
                    >
                        "Cancel"
                    </button>
                    <button
                        type="button"
                        on:click=save
                        disabled=move || saving.get()
                        class="px-4 py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm disabled:opacity-50"
                    >
                        {move || if saving.get() { "Saving..." } else { "Save" }}
                    </button>
                </div>
            </div>
        </Modal>
    }
}

//! `/admin/credentials` — vault CRUD UI (Phase 2b #12).
//!
//! Listing via `SortableTable<CredentialRow>`; per-row Reveal / Edit / Rename /
//! Delete actions dispatched through `CredentialPageActions` in Leptos context
//! (the trait `cell_view(&self, col)` has no slot for closures, so an
//! ambient-context handoff is the standard pattern here).
//!
//! Reveal is a strict two-step: confirm modal → server-fn → display modal with
//! a 30s text countdown driven by `gloo_timers::future::sleep`. Manual hide
//! and backdrop-click both clear the in-memory secret signal.
//!
//! Wire types live in `crate::admin_credentials::*` — DTOs only, no direct
//! `daimon-vault` import (D21).

use leptos::prelude::*;
use leptos::task::spawn_local;
use std::collections::BTreeMap;
#[cfg(feature = "hydrate")]
use std::time::Duration;

use crate::admin_credentials::{
    create_credential, delete_credential, list_credentials, rename_credential, reveal_credential,
    update_credential, CredentialDto, CredentialKindDto, CredentialRow,
};
use crate::components::modal::Modal;
use crate::components::sortable_table::{
    ColumnDef, SortType, SortableTable, TableRow,
};

// -------- Target + context for the actions column ----------------------------

#[derive(Clone, Debug)]
pub struct CredentialTarget {
    pub id: i64,
    pub name: String,
    pub kind: CredentialKindDto,
}

#[derive(Clone, Copy)]
struct CredentialPageActions {
    open_reveal: Callback<CredentialTarget>,
    open_edit: Callback<CredentialTarget>,
    open_rename: Callback<CredentialTarget>,
    open_delete: Callback<CredentialTarget>,
}

// -------- TableRow impl ------------------------------------------------------

impl TableRow for CredentialRow {
    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                key: "name",
                label: "Name",
                sortable: true,
                default_hidden: false,
                sort_type: SortType::Text,
            },
            ColumnDef {
                key: "kind",
                label: "Kind",
                sortable: true,
                default_hidden: false,
                sort_type: SortType::Text,
            },
            ColumnDef {
                key: "created_at",
                label: "Created",
                sortable: true,
                default_hidden: false,
                sort_type: SortType::Text,
            },
            ColumnDef {
                key: "updated_at",
                label: "Updated",
                sortable: true,
                default_hidden: false,
                sort_type: SortType::Text,
            },
            ColumnDef {
                key: "actions",
                label: "Actions",
                sortable: false,
                default_hidden: false,
                sort_type: SortType::Text,
            },
        ]
    }

    fn cell_value(&self, col: &str) -> String {
        match col {
            "name" => self.name.clone(),
            "kind" => self.kind.label().to_string(),
            "created_at" => self.created_at.clone(),
            "updated_at" => self.updated_at.clone(),
            _ => String::new(),
        }
    }

    fn cell_view(&self, col: &str) -> AnyView {
        match col {
            "name" => view! {
                <span class="text-text-primary font-medium">{self.name.clone()}</span>
            }
            .into_any(),
            "kind" => view! {
                <span class="inline-flex items-center px-2 py-0.5 rounded-full text-[11px] font-medium bg-surface-tertiary text-text-secondary border border-border-primary">
                    {self.kind.label()}
                </span>
            }
            .into_any(),
            "created_at" => view! {
                <span class="text-text-muted text-[12px]">{short_ts(&self.created_at)}</span>
            }
            .into_any(),
            "updated_at" => view! {
                <span class="text-text-muted text-[12px]">{short_ts(&self.updated_at)}</span>
            }
            .into_any(),
            "actions" => {
                let target = CredentialTarget {
                    id: self.id,
                    name: self.name.clone(),
                    kind: self.kind,
                };
                let actions = use_context::<CredentialPageActions>()
                    .expect("CredentialPageActions context must be provided");
                let t_reveal = target.clone();
                let t_edit = target.clone();
                let t_rename = target.clone();
                let t_delete = target;
                view! {
                    <div class="flex gap-1.5">
                        <ActionButton
                            label="Reveal".to_string()
                            on_click=Callback::new(move |_| actions.open_reveal.run(t_reveal.clone()))
                            danger=false
                        />
                        <ActionButton
                            label="Edit".to_string()
                            on_click=Callback::new(move |_| actions.open_edit.run(t_edit.clone()))
                            danger=false
                        />
                        <ActionButton
                            label="Rename".to_string()
                            on_click=Callback::new(move |_| actions.open_rename.run(t_rename.clone()))
                            danger=false
                        />
                        <ActionButton
                            label="Delete".to_string()
                            on_click=Callback::new(move |_| actions.open_delete.run(t_delete.clone()))
                            danger=true
                        />
                    </div>
                }
                .into_any()
            }
            _ => view! {}.into_any(),
        }
    }

    fn row_key(&self) -> String {
        self.id.to_string()
    }
}

#[component]
fn ActionButton(
    #[prop(into)] label: String,
    on_click: Callback<()>,
    #[prop(default = false)] danger: bool,
) -> impl IntoView {
    let label_view = label.clone();
    let class = if danger {
        "px-2 py-1 text-[12px] rounded text-accent-danger hover:bg-accent-danger/10 transition-colors"
    } else {
        "px-2 py-1 text-[12px] rounded text-text-secondary hover:text-text-primary hover:bg-surface-tertiary transition-colors"
    };
    view! {
        <button
            type="button"
            class=class
            on:click=move |_| { on_click.run(()); }
        >
            {label_view}
        </button>
    }
}

fn short_ts(rfc3339: &str) -> String {
    // Trim the seconds + timezone if the wire string is verbose. The wire
    // value is a full RFC3339 like 2026-05-22T03:14:00+08:00; show only
    // YYYY-MM-DD HH:MM for table density.
    if let Some((date, rest)) = rfc3339.split_once('T') {
        let hm = rest.get(0..5).unwrap_or(rest);
        format!("{} {}", date, hm)
    } else {
        rfc3339.to_string()
    }
}

// -------- Form draft state ---------------------------------------------------

#[derive(Clone, Debug)]
struct GenericRow {
    key: String,
    value: String,
}

#[derive(Clone, Debug)]
struct CredentialDraft {
    kind: CredentialKindDto,
    ssh_key_username: String,
    ssh_key_pem: String,
    ssh_key_passphrase: String,
    ssh_password_username: String,
    ssh_password_password: String,
    api_token: String,
    generic_rows: Vec<GenericRow>,
}

impl Default for CredentialDraft {
    fn default() -> Self {
        Self {
            kind: CredentialKindDto::SshKey,
            ssh_key_username: String::new(),
            ssh_key_pem: String::new(),
            ssh_key_passphrase: String::new(),
            ssh_password_username: String::new(),
            ssh_password_password: String::new(),
            api_token: String::new(),
            generic_rows: vec![GenericRow {
                key: String::new(),
                value: String::new(),
            }],
        }
    }
}

impl CredentialDraft {
    fn try_to_dto(&self) -> Result<CredentialDto, String> {
        match self.kind {
            CredentialKindDto::SshKey => {
                if self.ssh_key_username.trim().is_empty() {
                    return Err("Username is required".into());
                }
                if self.ssh_key_pem.trim().is_empty() {
                    return Err("Private key PEM is required".into());
                }
                let pass = if self.ssh_key_passphrase.is_empty() {
                    None
                } else {
                    Some(self.ssh_key_passphrase.clone())
                };
                Ok(CredentialDto::SshKey {
                    username: self.ssh_key_username.clone(),
                    private_key_pem: self.ssh_key_pem.clone(),
                    passphrase: pass,
                })
            }
            CredentialKindDto::SshPassword => {
                if self.ssh_password_username.trim().is_empty() {
                    return Err("Username is required".into());
                }
                if self.ssh_password_password.is_empty() {
                    return Err("Password is required".into());
                }
                Ok(CredentialDto::SshPassword {
                    username: self.ssh_password_username.clone(),
                    password: self.ssh_password_password.clone(),
                })
            }
            CredentialKindDto::ApiToken => {
                if self.api_token.is_empty() {
                    return Err("Token is required".into());
                }
                Ok(CredentialDto::ApiToken {
                    token: self.api_token.clone(),
                })
            }
            CredentialKindDto::Generic => {
                let mut fields = BTreeMap::new();
                for row in &self.generic_rows {
                    let k = row.key.trim();
                    if k.is_empty() {
                        continue;
                    }
                    fields.insert(k.to_string(), row.value.clone());
                }
                if fields.is_empty() {
                    return Err("At least one key/value pair is required".into());
                }
                Ok(CredentialDto::Generic { fields })
            }
        }
    }
}

// -------- Page component ----------------------------------------------------

#[component]
pub fn AdminCredentials() -> impl IntoView {
    // Refresh counter — bumping it triggers Resource refetch.
    let refresh = RwSignal::new(0u64);

    let credentials_res = Resource::new(
        move || refresh.get(),
        |_| async move { list_credentials().await },
    );

    // Modal open signals + companion target signals.
    let add_open = RwSignal::new(false);
    let edit_open = RwSignal::new(false);
    let edit_target = RwSignal::new(None::<CredentialTarget>);
    let rename_open = RwSignal::new(false);
    let rename_target = RwSignal::new(None::<CredentialTarget>);
    let delete_open = RwSignal::new(false);
    let delete_target = RwSignal::new(None::<CredentialTarget>);
    let reveal_confirm_open = RwSignal::new(false);
    let reveal_target = RwSignal::new(None::<CredentialTarget>);
    let reveal_display_open = RwSignal::new(false);
    let revealed_secret = RwSignal::new(None::<CredentialDto>);
    let countdown = RwSignal::new(30u32);

    // Action handlers exposed via context for the per-row buttons.
    let actions = CredentialPageActions {
        open_reveal: Callback::new(move |t: CredentialTarget| {
            reveal_target.set(Some(t));
            reveal_confirm_open.set(true);
        }),
        open_edit: Callback::new(move |t: CredentialTarget| {
            edit_target.set(Some(t));
            edit_open.set(true);
        }),
        open_rename: Callback::new(move |t: CredentialTarget| {
            rename_target.set(Some(t));
            rename_open.set(true);
        }),
        open_delete: Callback::new(move |t: CredentialTarget| {
            delete_target.set(Some(t));
            delete_open.set(true);
        }),
    };
    provide_context(actions);

    // 30s countdown effect — drives auto-hide on reveal display. Only runs
    // under the hydrate target (gloo-timers is a wasm-only crate).
    Effect::new(move |prev: Option<bool>| {
        let open = reveal_display_open.get();
        let was_open = prev.unwrap_or(false);
        if open && !was_open {
            countdown.set(30);
            #[cfg(feature = "hydrate")]
            spawn_local(async move {
                loop {
                    gloo_timers::future::sleep(Duration::from_secs(1)).await;
                    if !reveal_display_open.get_untracked() {
                        return;
                    }
                    let remaining = countdown.get_untracked().saturating_sub(1);
                    countdown.set(remaining);
                    if remaining == 0 {
                        revealed_secret.set(None);
                        reveal_display_open.set(false);
                        return;
                    }
                }
            });
        }
        open
    });

    // Clear the secret signal when the display modal closes by any means.
    Effect::new(move |prev: Option<bool>| {
        let open = reveal_display_open.get();
        let was_open = prev.unwrap_or(false);
        if was_open && !open {
            revealed_secret.set(None);
        }
        open
    });

    view! {
        <div>
            <div class="flex items-center justify-between mb-6">
                <h1 class="text-xl font-semibold text-text-primary">"Credentials"</h1>
                <button
                    type="button"
                    on:click=move |_| add_open.set(true)
                    class="px-4 py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm"
                >
                    "Add Credential"
                </button>
            </div>

            <Suspense fallback=|| view! {
                <p class="text-text-muted text-sm">"Loading credentials..."</p>
            }>
                {move || credentials_res.get().map(|result| match result {
                    Ok(rows) if rows.is_empty() => view! {
                        <p class="text-text-muted text-sm">"No credentials yet. Click \"Add Credential\" to create one."</p>
                    }.into_any(),
                    Ok(rows) => view! {
                        <SortableTable<CredentialRow> rows=rows table_id="admin-credentials" />
                    }.into_any(),
                    Err(e) => view! {
                        <p class="text-accent-danger text-sm">{e.to_string()}</p>
                    }.into_any(),
                })}
            </Suspense>

            <AddModal open=add_open refresh=refresh />
            <EditModal open=edit_open target=edit_target refresh=refresh />
            <RenameModal open=rename_open target=rename_target refresh=refresh />
            <DeleteModal open=delete_open target=delete_target refresh=refresh />
            <RevealConfirmModal
                open=reveal_confirm_open
                target=reveal_target
                display_open=reveal_display_open
                revealed_secret=revealed_secret
            />
            <RevealDisplayModal
                open=reveal_display_open
                target=reveal_target
                secret=revealed_secret
                countdown=countdown
            />
        </div>
    }
}

// -------- Modals ------------------------------------------------------------

#[component]
fn AddModal(open: RwSignal<bool>, refresh: RwSignal<u64>) -> impl IntoView {
    let name = RwSignal::new(String::new());
    let draft = RwSignal::new(CredentialDraft::default());
    let error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);

    // Reset state when the modal opens.
    Effect::new(move |prev: Option<bool>| {
        let is_open = open.get();
        let was_open = prev.unwrap_or(false);
        if is_open && !was_open {
            name.set(String::new());
            draft.set(CredentialDraft::default());
            error.set(None);
            saving.set(false);
        }
        is_open
    });

    let save = move |_| {
        let n = name.get_untracked();
        if n.trim().is_empty() {
            error.set(Some("Name is required".into()));
            return;
        }
        let dto = match draft.get_untracked().try_to_dto() {
            Ok(d) => d,
            Err(e) => {
                error.set(Some(e));
                return;
            }
        };
        error.set(None);
        saving.set(true);
        spawn_local(async move {
            let result = create_credential(n, dto).await;
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
        <Modal title="Add Credential".to_string() open=open max_width="max-w-xl">
            <div class="space-y-4">
                <div>
                    <label class="block text-sm text-text-secondary mb-1">"Name"</label>
                    <input
                        type="text"
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        placeholder="mikrotik-edge"
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))
                    />
                </div>
                <CredentialEditor draft=draft />
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

#[component]
fn EditModal(
    open: RwSignal<bool>,
    target: RwSignal<Option<CredentialTarget>>,
    refresh: RwSignal<u64>,
) -> impl IntoView {
    let draft = RwSignal::new(CredentialDraft::default());
    let error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);

    Effect::new(move |prev: Option<bool>| {
        let is_open = open.get();
        let was_open = prev.unwrap_or(false);
        if is_open && !was_open {
            // Preload the draft to the target's kind so the right form shows;
            // values stay blank — we never re-display stored secrets.
            if let Some(t) = target.get_untracked() {
                let mut d = CredentialDraft::default();
                d.kind = t.kind;
                draft.set(d);
            }
            error.set(None);
            saving.set(false);
        }
        is_open
    });

    let save = move |_| {
        let Some(t) = target.get_untracked() else { return };
        let dto = match draft.get_untracked().try_to_dto() {
            Ok(d) => d,
            Err(e) => {
                error.set(Some(e));
                return;
            }
        };
        error.set(None);
        saving.set(true);
        let id = t.id;
        spawn_local(async move {
            let result = update_credential(id, dto).await;
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
        <Modal title="Edit Credential".to_string() open=open max_width="max-w-xl">
            <div class="space-y-4">
                <div>
                    <label class="block text-sm text-text-secondary mb-1">"Name"</label>
                    <input
                        type="text"
                        disabled
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-muted text-sm opacity-60 cursor-not-allowed"
                        prop:value=move || target.get().map(|t| t.name).unwrap_or_default()
                    />
                    <p class="text-text-muted text-xs mt-1">"Use Rename to change the name."</p>
                </div>
                <CredentialEditor draft=draft kind_locked=true />
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

#[component]
fn RenameModal(
    open: RwSignal<bool>,
    target: RwSignal<Option<CredentialTarget>>,
    refresh: RwSignal<u64>,
) -> impl IntoView {
    let new_name = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);

    Effect::new(move |prev: Option<bool>| {
        let is_open = open.get();
        let was_open = prev.unwrap_or(false);
        if is_open && !was_open {
            new_name.set(target.get_untracked().map(|t| t.name).unwrap_or_default());
            error.set(None);
            saving.set(false);
        }
        is_open
    });

    let save = move |_| {
        let Some(t) = target.get_untracked() else { return };
        let nn = new_name.get_untracked();
        if nn.trim().is_empty() {
            error.set(Some("Name is required".into()));
            return;
        }
        if nn == t.name {
            open.set(false);
            return;
        }
        saving.set(true);
        let id = t.id;
        spawn_local(async move {
            let result = rename_credential(id, nn).await;
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
        <Modal title="Rename Credential".to_string() open=open>
            <div class="space-y-4">
                <div>
                    <label class="block text-sm text-text-secondary mb-1">"New Name"</label>
                    <input
                        type="text"
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        prop:value=move || new_name.get()
                        on:input=move |ev| new_name.set(event_target_value(&ev))
                    />
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

#[component]
fn DeleteModal(
    open: RwSignal<bool>,
    target: RwSignal<Option<CredentialTarget>>,
    refresh: RwSignal<u64>,
) -> impl IntoView {
    let error = RwSignal::new(None::<String>);
    let deleting = RwSignal::new(false);

    Effect::new(move |prev: Option<bool>| {
        let is_open = open.get();
        let was_open = prev.unwrap_or(false);
        if is_open && !was_open {
            error.set(None);
            deleting.set(false);
        }
        is_open
    });

    let confirm = move |_| {
        let Some(t) = target.get_untracked() else { return };
        deleting.set(true);
        let id = t.id;
        spawn_local(async move {
            let result = delete_credential(id).await;
            deleting.set(false);
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
        <Modal title="Delete Credential".to_string() open=open>
            <div class="space-y-4">
                <p class="text-sm text-text-primary">
                    "Delete "
                    <span class="font-semibold">{move || target.get().map(|t| t.name).unwrap_or_default()}</span>
                    "?"
                </p>
                <p class="text-xs text-text-muted">
                    "This is permanent. Any target referencing this credential will fail to resolve secrets."
                </p>
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
                        on:click=confirm
                        disabled=move || deleting.get()
                        class="px-4 py-2 bg-accent-danger text-surface-primary font-medium rounded-md hover:bg-accent-danger/90 transition-colors text-sm disabled:opacity-50"
                    >
                        {move || if deleting.get() { "Deleting..." } else { "Delete" }}
                    </button>
                </div>
            </div>
        </Modal>
    }
}

#[component]
fn RevealConfirmModal(
    open: RwSignal<bool>,
    target: RwSignal<Option<CredentialTarget>>,
    display_open: RwSignal<bool>,
    revealed_secret: RwSignal<Option<CredentialDto>>,
) -> impl IntoView {
    let error = RwSignal::new(None::<String>);
    let revealing = RwSignal::new(false);

    Effect::new(move |prev: Option<bool>| {
        let is_open = open.get();
        let was_open = prev.unwrap_or(false);
        if is_open && !was_open {
            error.set(None);
            revealing.set(false);
        }
        is_open
    });

    let confirm = move |_| {
        let Some(t) = target.get_untracked() else { return };
        revealing.set(true);
        let id = t.id;
        spawn_local(async move {
            let result = reveal_credential(id).await;
            revealing.set(false);
            match result {
                Ok(secret) => {
                    revealed_secret.set(Some(secret));
                    open.set(false);
                    display_open.set(true);
                }
                Err(e) => error.set(Some(format!("{e}"))),
            }
        });
    };

    view! {
        <Modal title="Reveal Credential".to_string() open=open>
            <div class="space-y-4">
                <p class="text-sm text-text-primary">
                    "Reveal "
                    <span class="font-semibold">{move || target.get().map(|t| t.name).unwrap_or_default()}</span>
                    "?"
                </p>
                <p class="text-xs text-text-muted">
                    "This is the most audited action. The reveal will be logged to the audit trail and tied to your username. The secret will auto-hide after 30 seconds."
                </p>
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
                        on:click=confirm
                        disabled=move || revealing.get()
                        class="px-4 py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm disabled:opacity-50"
                    >
                        {move || if revealing.get() { "Revealing..." } else { "Reveal" }}
                    </button>
                </div>
            </div>
        </Modal>
    }
}

#[component]
fn RevealDisplayModal(
    open: RwSignal<bool>,
    target: RwSignal<Option<CredentialTarget>>,
    secret: RwSignal<Option<CredentialDto>>,
    countdown: RwSignal<u32>,
) -> impl IntoView {
    let copied = RwSignal::new(false);

    let copy_all = move |_| {
        let Some(s) = secret.get_untracked() else { return };
        let text = format_secret_for_clipboard(&s);
        copy_to_clipboard(text);
        copied.set(true);
        #[cfg(feature = "hydrate")]
        spawn_local(async move {
            gloo_timers::future::sleep(Duration::from_secs(2)).await;
            copied.set(false);
        });
    };

    view! {
        <Modal title="Revealed Credential".to_string() open=open max_width="max-w-2xl">
            <div class="space-y-4">
                <div class="flex items-center justify-between">
                    <div class="text-sm text-text-secondary">
                        <span class="text-text-muted">"Name: "</span>
                        <span class="text-text-primary font-medium">
                            {move || target.get().map(|t| t.name).unwrap_or_default()}
                        </span>
                    </div>
                    <div class="inline-flex items-center px-2 py-0.5 rounded-full text-[11px] font-medium bg-accent-amber/10 text-accent-amber border border-accent-amber/30">
                        "Hides in "{move || countdown.get()}"s"
                    </div>
                </div>

                {move || secret.get().map(|s| render_secret(&s))}

                <div class="flex gap-3 justify-end pt-2">
                    <button
                        type="button"
                        on:click=copy_all
                        class="px-4 py-2 bg-surface-tertiary border border-border-primary rounded-md hover:bg-surface-tertiary/80 transition-colors text-sm text-text-secondary"
                    >
                        {move || if copied.get() { "Copied!" } else { "Copy" }}
                    </button>
                    <button
                        type="button"
                        on:click=move |_| open.set(false)
                        class="px-4 py-2 bg-accent-danger text-surface-primary font-medium rounded-md hover:bg-accent-danger/90 transition-colors text-sm"
                    >
                        "Hide Now"
                    </button>
                </div>
            </div>
        </Modal>
    }
}

fn render_secret(s: &CredentialDto) -> AnyView {
    match s {
        CredentialDto::SshKey { username, private_key_pem, passphrase } => {
            let username = username.clone();
            let pem = private_key_pem.clone();
            let pass = passphrase.clone().unwrap_or_default();
            let has_pass = !pass.is_empty();
            view! {
                <div class="space-y-3">
                    <SecretField label="Username".to_string() value=username monospace=false />
                    <SecretField label="Private Key PEM".to_string() value=pem monospace=true />
                    <Show when=move || has_pass>
                        <SecretField label="Passphrase".to_string() value=pass.clone() monospace=true />
                    </Show>
                </div>
            }.into_any()
        }
        CredentialDto::SshPassword { username, password } => {
            let username = username.clone();
            let password = password.clone();
            view! {
                <div class="space-y-3">
                    <SecretField label="Username".to_string() value=username monospace=false />
                    <SecretField label="Password".to_string() value=password monospace=true />
                </div>
            }.into_any()
        }
        CredentialDto::ApiToken { token } => {
            let token = token.clone();
            view! {
                <SecretField label="Token".to_string() value=token monospace=true />
            }.into_any()
        }
        CredentialDto::Generic { fields } => {
            let rows: Vec<_> = fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            view! {
                <div class="space-y-3">
                    {rows.into_iter().map(|(k, v)| view! {
                        <SecretField label=k value=v monospace=true />
                    }).collect_view()}
                </div>
            }.into_any()
        }
    }
}

#[component]
fn SecretField(
    #[prop(into)] label: String,
    #[prop(into)] value: String,
    #[prop(default = false)] monospace: bool,
) -> impl IntoView {
    let label_view = label.clone();
    let value_for_view = value.clone();
    let class = if monospace {
        "w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-xs font-mono whitespace-pre-wrap break-all min-h-[2.5rem] max-h-48 overflow-y-auto"
    } else {
        "w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm"
    };
    view! {
        <div>
            <label class="block text-xs text-text-muted mb-1">{label_view}</label>
            <div class=class>{value_for_view}</div>
        </div>
    }
}

fn format_secret_for_clipboard(s: &CredentialDto) -> String {
    match s {
        CredentialDto::SshKey { username, private_key_pem, passphrase } => {
            let mut buf = format!("username: {}\nprivate_key_pem:\n{}", username, private_key_pem);
            if let Some(p) = passphrase {
                buf.push_str(&format!("\npassphrase: {}", p));
            }
            buf
        }
        CredentialDto::SshPassword { username, password } => {
            format!("username: {}\npassword: {}", username, password)
        }
        CredentialDto::ApiToken { token } => token.clone(),
        CredentialDto::Generic { fields } => fields
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn copy_to_clipboard(text: String) {
    #[cfg(feature = "hydrate")]
    {
        if let Some(window) = web_sys::window() {
            let clipboard = window.navigator().clipboard();
            let _ = clipboard.write_text(&text);
        }
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = text;
    }
}

// -------- Credential editor (sub-component shared by Add + Edit) -------------

#[component]
fn CredentialEditor(
    draft: RwSignal<CredentialDraft>,
    #[prop(default = false)] kind_locked: bool,
) -> impl IntoView {
    view! {
        <div class="space-y-3">
            <div>
                <label class="block text-sm text-text-secondary mb-1">
                    "Kind"
                    {if kind_locked {
                        view! {
                            <span class="ml-2 text-text-muted text-[11px]">"(fixed; delete + recreate to change kind)"</span>
                        }.into_any()
                    } else {
                        view! {}.into_any()
                    }}
                </label>
                <div class="flex gap-1 p-1 bg-surface-tertiary rounded-md border border-border-primary">
                    {[
                        CredentialKindDto::SshKey,
                        CredentialKindDto::SshPassword,
                        CredentialKindDto::ApiToken,
                        CredentialKindDto::Generic,
                    ].iter().map(|k| {
                        let kind = *k;
                        view! {
                            <button
                                type="button"
                                disabled=kind_locked
                                on:click=move |_| {
                                    if !kind_locked {
                                        draft.update(|d| d.kind = kind);
                                    }
                                }
                                class=move || {
                                    let active = draft.get().kind == kind;
                                    let base = "flex-1 px-3 py-1.5 text-[12px] rounded transition-colors";
                                    if active {
                                        format!("{} bg-accent-amber text-surface-primary font-medium", base)
                                    } else if kind_locked {
                                        format!("{} text-text-muted opacity-40 cursor-not-allowed", base)
                                    } else {
                                        format!("{} text-text-secondary hover:text-text-primary", base)
                                    }
                                }
                            >
                                {kind.label()}
                            </button>
                        }
                    }).collect_view()}
                </div>
            </div>

            {move || match draft.get().kind {
                CredentialKindDto::SshKey => view! { <SshKeyFields draft=draft /> }.into_any(),
                CredentialKindDto::SshPassword => view! { <SshPasswordFields draft=draft /> }.into_any(),
                CredentialKindDto::ApiToken => view! { <ApiTokenFields draft=draft /> }.into_any(),
                CredentialKindDto::Generic => view! { <GenericFields draft=draft /> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn SshKeyFields(draft: RwSignal<CredentialDraft>) -> impl IntoView {
    let on_file_change = move |_ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::closure::Closure;
            use wasm_bindgen::JsCast;

            let Some(target) = _ev.target() else { return };
            let Ok(input) = target.dyn_into::<web_sys::HtmlInputElement>() else { return };
            let Some(files) = input.files() else { return };
            let Some(file) = files.get(0) else { return };

            let Ok(reader) = web_sys::FileReader::new() else { return };
            let reader_clone = reader.clone();
            let onload = Closure::wrap(Box::new(move |_: web_sys::ProgressEvent| {
                if let Ok(value) = reader_clone.result() {
                    if let Some(text) = value.as_string() {
                        draft.update(|d| d.ssh_key_pem = text);
                    }
                }
            }) as Box<dyn FnMut(web_sys::ProgressEvent)>);
            reader.set_onload(Some(onload.as_ref().unchecked_ref()));
            onload.forget();
            let _ = reader.read_as_text(&file);
            // Clear the input value so the same file can be re-picked later.
            input.set_value("");
        }
    };

    view! {
        <div class="space-y-3">
            <div>
                <label class="block text-sm text-text-secondary mb-1">"Username"</label>
                <input
                    type="text"
                    class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                    placeholder="root"
                    prop:value=move || draft.get().ssh_key_username
                    on:input=move |ev| draft.update(|d| d.ssh_key_username = event_target_value(&ev))
                />
            </div>
            <div>
                <div class="flex items-center justify-between mb-1">
                    <label class="block text-sm text-text-secondary">"Private Key (PEM)"</label>
                    <label class="cursor-pointer text-xs text-accent-amber hover:underline">
                        "Upload from file"
                        <input
                            type="file"
                            class="hidden"
                            on:change=on_file_change
                        />
                    </label>
                </div>
                <textarea
                    rows="8"
                    class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-xs font-mono focus:outline-none focus:border-accent-amber"
                    placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
                    prop:value=move || draft.get().ssh_key_pem
                    on:input=move |ev| draft.update(|d| d.ssh_key_pem = event_target_value(&ev))
                ></textarea>
            </div>
            <div>
                <label class="block text-sm text-text-secondary mb-1">"Passphrase (optional)"</label>
                <input
                    type="password"
                    class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                    prop:value=move || draft.get().ssh_key_passphrase
                    on:input=move |ev| draft.update(|d| d.ssh_key_passphrase = event_target_value(&ev))
                />
            </div>
        </div>
    }
}

#[component]
fn SshPasswordFields(draft: RwSignal<CredentialDraft>) -> impl IntoView {
    view! {
        <div class="space-y-3">
            <div>
                <label class="block text-sm text-text-secondary mb-1">"Username"</label>
                <input
                    type="text"
                    class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                    placeholder="admin"
                    prop:value=move || draft.get().ssh_password_username
                    on:input=move |ev| draft.update(|d| d.ssh_password_username = event_target_value(&ev))
                />
            </div>
            <div>
                <label class="block text-sm text-text-secondary mb-1">"Password"</label>
                <input
                    type="password"
                    class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                    prop:value=move || draft.get().ssh_password_password
                    on:input=move |ev| draft.update(|d| d.ssh_password_password = event_target_value(&ev))
                />
            </div>
        </div>
    }
}

#[component]
fn ApiTokenFields(draft: RwSignal<CredentialDraft>) -> impl IntoView {
    view! {
        <div>
            <label class="block text-sm text-text-secondary mb-1">"Token"</label>
            <input
                type="password"
                class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm font-mono focus:outline-none focus:border-accent-amber"
                placeholder="user@realm!token=xxxx"
                prop:value=move || draft.get().api_token
                on:input=move |ev| draft.update(|d| d.api_token = event_target_value(&ev))
            />
        </div>
    }
}

#[component]
fn GenericFields(draft: RwSignal<CredentialDraft>) -> impl IntoView {
    let add_row = move |_| {
        draft.update(|d| {
            d.generic_rows.push(GenericRow {
                key: String::new(),
                value: String::new(),
            });
        });
    };

    view! {
        <div class="space-y-2">
            <label class="block text-sm text-text-secondary">"Key/Value Fields"</label>
            <div class="space-y-2">
                {move || {
                    let rows = draft.get().generic_rows;
                    rows.into_iter().enumerate().map(|(idx, row)| {
                        view! {
                            <div class="flex gap-2">
                                <input
                                    type="text"
                                    class="flex-1 px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                                    placeholder="key"
                                    prop:value=row.key.clone()
                                    on:input=move |ev| draft.update(|d| {
                                        if let Some(r) = d.generic_rows.get_mut(idx) {
                                            r.key = event_target_value(&ev);
                                        }
                                    })
                                />
                                <input
                                    type="password"
                                    class="flex-1 px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm font-mono focus:outline-none focus:border-accent-amber"
                                    placeholder="value"
                                    prop:value=row.value.clone()
                                    on:input=move |ev| draft.update(|d| {
                                        if let Some(r) = d.generic_rows.get_mut(idx) {
                                            r.value = event_target_value(&ev);
                                        }
                                    })
                                />
                                <button
                                    type="button"
                                    on:click=move |_| draft.update(|d| { d.generic_rows.remove(idx); })
                                    class="px-3 py-2 text-text-muted hover:text-accent-danger transition-colors text-sm"
                                    title="Remove row"
                                >
                                    "−"
                                </button>
                            </div>
                        }
                    }).collect_view()
                }}
            </div>
            <button
                type="button"
                on:click=add_row
                class="text-xs text-accent-amber hover:underline"
            >
                "+ Add Field"
            </button>
        </div>
    }
}

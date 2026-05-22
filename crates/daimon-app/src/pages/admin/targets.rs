//! `/admin/targets` — managed-target CRUD UI (Phase 2b #13).
//!
//! Same shape as `/admin/credentials`: SortableTable list + per-row actions
//! dispatched through `TargetPageActions` in Leptos context, Add/Edit/Delete
//! modals (no rename — ref is identity, broker has no rename op; delete +
//! recreate is the path).
//!
//! Edit fetches the full record via `get_target()` (server-side calls
//! `broker.inventory_get_managed` which is audited as `InventoryResolve`).
//! Add posts via `upsert_target()`; Delete via `delete_target()`.
//!
//! Credential picker sourced from `crate::admin_credentials::list_credentials`
//! — same vault entries the credentials page manages.

use leptos::prelude::*;
use leptos::task::spawn_local;
use std::collections::BTreeMap;

use crate::admin_credentials::{list_credentials, CredentialRow};
use crate::admin_targets::{
    delete_target, get_target, list_targets, upsert_target, TargetDto, TargetKindDto, TargetRow,
    TransportKindDto,
};
use crate::components::modal::Modal;
use crate::components::sortable_table::{ColumnDef, SortType, SortableTable, TableRow};

// -------- Target action context (for per-row buttons) ------------------------

#[derive(Clone)]
pub struct TargetActionTarget {
    pub ref_name: String,
}

#[derive(Clone, Copy)]
struct TargetPageActions {
    open_edit: Callback<TargetActionTarget>,
    open_delete: Callback<TargetActionTarget>,
}

// -------- TableRow impl ------------------------------------------------------

impl TableRow for TargetRow {
    fn columns() -> Vec<ColumnDef> {
        vec![
            ColumnDef {
                key: "ref",
                label: "Ref",
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
                key: "transport",
                label: "Transport",
                sortable: true,
                default_hidden: false,
                sort_type: SortType::Text,
            },
            ColumnDef {
                key: "endpoint",
                label: "Host:Port",
                sortable: true,
                default_hidden: false,
                sort_type: SortType::Text,
            },
            ColumnDef {
                key: "labels",
                label: "Labels",
                sortable: true,
                default_hidden: false,
                sort_type: SortType::Numeric,
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
            "ref" => self.ref_name.clone(),
            "kind" => self.kind.label().to_string(),
            "transport" => self.transport.label().to_string(),
            "endpoint" => format!("{}:{}", self.host, self.port),
            "labels" => self.label_count.to_string(),
            _ => String::new(),
        }
    }

    fn cell_view(&self, col: &str) -> AnyView {
        match col {
            "ref" => view! {
                <span class="text-text-primary font-mono text-[12px]">"target://"{self.ref_name.clone()}</span>
            }
            .into_any(),
            "kind" => view! {
                <span class="inline-flex items-center px-2 py-0.5 rounded-full text-[11px] font-medium bg-surface-tertiary text-text-secondary border border-border-primary">
                    {self.kind.label()}
                </span>
            }
            .into_any(),
            "transport" => view! {
                <span class="inline-flex items-center px-2 py-0.5 rounded-full text-[11px] font-medium bg-surface-tertiary text-text-secondary border border-border-primary">
                    {self.transport.label()}
                </span>
            }
            .into_any(),
            "endpoint" => view! {
                <span class="text-text-secondary text-[12px] font-mono">{format!("{}:{}", self.host, self.port)}</span>
            }
            .into_any(),
            "labels" => {
                let count = self.label_count;
                view! {
                    <span class="text-text-muted text-[12px]">{count}</span>
                }
                .into_any()
            }
            "actions" => {
                let target = TargetActionTarget {
                    ref_name: self.ref_name.clone(),
                };
                let actions = use_context::<TargetPageActions>()
                    .expect("TargetPageActions context must be provided");
                let t_edit = target.clone();
                let t_delete = target;
                view! {
                    <div class="flex gap-1.5">
                        <ActionButton
                            label="Edit".to_string()
                            on_click=Callback::new(move |_| actions.open_edit.run(t_edit.clone()))
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
        self.ref_name.clone()
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

// -------- Form draft state ---------------------------------------------------

#[derive(Clone, Debug)]
struct LabelRow {
    key: String,
    value: String,
}

#[derive(Clone, Debug)]
struct TargetDraft {
    ref_name: String,
    kind: TargetKindDto,
    transport: TransportKindDto,
    host: String,
    port: String,
    port_touched: bool,
    credential_ref: String,
    labels: Vec<LabelRow>,
    capabilities: Vec<String>,
}

impl Default for TargetDraft {
    fn default() -> Self {
        Self {
            ref_name: String::new(),
            kind: TargetKindDto::Host,
            transport: TransportKindDto::Ssh,
            host: String::new(),
            port: TransportKindDto::Ssh.default_port().to_string(),
            port_touched: false,
            credential_ref: String::new(),
            labels: vec![],
            capabilities: vec![],
        }
    }
}

impl TargetDraft {
    fn from_dto(dto: TargetDto) -> Self {
        Self {
            ref_name: dto.ref_name,
            kind: dto.kind,
            transport: dto.transport,
            host: dto.host,
            port: dto.port.to_string(),
            port_touched: true,
            credential_ref: dto.credential_ref,
            labels: dto
                .labels
                .into_iter()
                .map(|(key, value)| LabelRow { key, value })
                .collect(),
            capabilities: dto.capabilities,
        }
    }

    fn to_dto(&self) -> Result<TargetDto, String> {
        if self.ref_name.trim().is_empty() {
            return Err("Ref name is required".into());
        }
        if self.host.trim().is_empty() {
            return Err("Host is required".into());
        }
        let port: u16 = self
            .port
            .trim()
            .parse()
            .map_err(|_| "Port must be a number between 1 and 65535".to_string())?;
        let mut labels = BTreeMap::new();
        for row in &self.labels {
            let k = row.key.trim();
            if k.is_empty() {
                continue;
            }
            labels.insert(k.to_string(), row.value.clone());
        }
        Ok(TargetDto {
            ref_name: self.ref_name.trim().to_string(),
            kind: self.kind,
            transport: self.transport,
            host: self.host.trim().to_string(),
            port,
            credential_ref: self.credential_ref.trim().to_string(),
            labels,
            capabilities: self.capabilities.clone(),
        })
    }
}

// -------- Page component -----------------------------------------------------

#[component]
pub fn AdminTargets() -> impl IntoView {
    let refresh = RwSignal::new(0u64);

    let targets_res = Resource::new(
        move || refresh.get(),
        |_| async move { list_targets().await },
    );

    let credentials_res = Resource::new(
        move || refresh.get(),
        |_| async move { list_credentials().await },
    );

    let add_open = RwSignal::new(false);
    let edit_open = RwSignal::new(false);
    let edit_target = RwSignal::new(None::<TargetActionTarget>);
    let delete_open = RwSignal::new(false);
    let delete_target = RwSignal::new(None::<TargetActionTarget>);

    let actions = TargetPageActions {
        open_edit: Callback::new(move |t: TargetActionTarget| {
            edit_target.set(Some(t));
            edit_open.set(true);
        }),
        open_delete: Callback::new(move |t: TargetActionTarget| {
            delete_target.set(Some(t));
            delete_open.set(true);
        }),
    };
    provide_context(actions);

    view! {
        <div>
            <div class="flex items-center justify-between mb-6">
                <h1 class="text-xl font-semibold text-text-primary">"Targets"</h1>
                <button
                    type="button"
                    on:click=move |_| add_open.set(true)
                    class="px-4 py-2 bg-accent-amber text-surface-primary font-medium rounded-md hover:bg-accent-amber/90 transition-colors text-sm"
                >
                    "Add Target"
                </button>
            </div>

            <Suspense fallback=|| view! {
                <p class="text-text-muted text-sm">"Loading targets..."</p>
            }>
                {move || targets_res.get().map(|result| match result {
                    Ok(rows) if rows.is_empty() => view! {
                        <p class="text-text-muted text-sm">"No targets yet. Click \"Add Target\" to register one."</p>
                    }.into_any(),
                    Ok(rows) => view! {
                        <SortableTable<TargetRow> rows=rows table_id="admin-targets" />
                    }.into_any(),
                    Err(e) => view! {
                        <p class="text-accent-danger text-sm">{e.to_string()}</p>
                    }.into_any(),
                })}
            </Suspense>

            <AddModal open=add_open refresh=refresh credentials_res=credentials_res />
            <EditModal open=edit_open target=edit_target refresh=refresh credentials_res=credentials_res />
            <DeleteModal open=delete_open target=delete_target refresh=refresh />
        </div>
    }
}

// -------- Modals ------------------------------------------------------------

type CredentialsResource = Resource<Result<Vec<CredentialRow>, ServerFnError>>;

#[component]
fn AddModal(
    open: RwSignal<bool>,
    refresh: RwSignal<u64>,
    credentials_res: CredentialsResource,
) -> impl IntoView {
    let draft = RwSignal::new(TargetDraft::default());
    let error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);

    Effect::new(move |prev: Option<bool>| {
        let is_open = open.get();
        let was_open = prev.unwrap_or(false);
        if is_open && !was_open {
            draft.set(TargetDraft::default());
            error.set(None);
            saving.set(false);
        }
        is_open
    });

    let save = move |_| {
        let dto = match draft.get_untracked().to_dto() {
            Ok(d) => d,
            Err(e) => {
                error.set(Some(e));
                return;
            }
        };
        error.set(None);
        saving.set(true);
        spawn_local(async move {
            let result = upsert_target(dto).await;
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
        <Modal title="Add Target".to_string() open=open max_width="max-w-xl">
            <div class="space-y-4">
                <TargetEditor draft=draft credentials_res=credentials_res ref_locked=false />
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
    target: RwSignal<Option<TargetActionTarget>>,
    refresh: RwSignal<u64>,
    credentials_res: CredentialsResource,
) -> impl IntoView {
    let draft = RwSignal::new(TargetDraft::default());
    let error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);
    let loading = RwSignal::new(false);

    Effect::new(move |prev: Option<bool>| {
        let is_open = open.get();
        let was_open = prev.unwrap_or(false);
        if is_open && !was_open {
            error.set(None);
            saving.set(false);
            // Fetch full record to populate the form.
            if let Some(t) = target.get_untracked() {
                loading.set(true);
                let ref_name = t.ref_name.clone();
                spawn_local(async move {
                    match get_target(ref_name).await {
                        Ok(dto) => {
                            draft.set(TargetDraft::from_dto(dto));
                            loading.set(false);
                        }
                        Err(e) => {
                            error.set(Some(format!("{e}")));
                            loading.set(false);
                        }
                    }
                });
            }
        }
        is_open
    });

    let save = move |_| {
        let dto = match draft.get_untracked().to_dto() {
            Ok(d) => d,
            Err(e) => {
                error.set(Some(e));
                return;
            }
        };
        error.set(None);
        saving.set(true);
        spawn_local(async move {
            let result = upsert_target(dto).await;
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
        <Modal title="Edit Target".to_string() open=open max_width="max-w-xl">
            {move || if loading.get() {
                view! {
                    <p class="text-text-muted text-sm">"Loading target..."</p>
                }.into_any()
            } else {
                view! {
                    <div class="space-y-4">
                        <TargetEditor draft=draft credentials_res=credentials_res ref_locked=true />
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
                }.into_any()
            }}
        </Modal>
    }
}

#[component]
fn DeleteModal(
    open: RwSignal<bool>,
    target: RwSignal<Option<TargetActionTarget>>,
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
        let ref_name = t.ref_name.clone();
        spawn_local(async move {
            let result = delete_target(ref_name).await;
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
        <Modal title="Delete Target".to_string() open=open>
            <div class="space-y-4">
                <p class="text-sm text-text-primary">
                    "Delete "
                    <span class="font-mono font-semibold">"target://"{move || target.get().map(|t| t.ref_name).unwrap_or_default()}</span>
                    "?"
                </p>
                <p class="text-xs text-text-muted">
                    "Permanent. Any active operation referencing this target will fail to resolve."
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

// -------- Target editor sub-component ---------------------------------------

#[component]
fn TargetEditor(
    draft: RwSignal<TargetDraft>,
    credentials_res: CredentialsResource,
    #[prop(default = false)] ref_locked: bool,
) -> impl IntoView {
    // When transport changes, auto-update port if the user hasn't touched it.
    let on_transport_change = move |t: TransportKindDto| {
        draft.update(|d| {
            d.transport = t;
            if !d.port_touched {
                d.port = t.default_port().to_string();
            }
        });
    };

    let add_label = move |_| {
        draft.update(|d| {
            d.labels.push(LabelRow {
                key: String::new(),
                value: String::new(),
            });
        });
    };

    view! {
        <div class="space-y-3">
            // Ref name
            <div>
                <label class="block text-sm text-text-secondary mb-1">
                    "Ref name"
                    {if ref_locked {
                        view! {
                            <span class="ml-2 text-text-muted text-[11px]">"(fixed; delete + recreate to change)"</span>
                        }.into_any()
                    } else {
                        view! {
                            <span class="ml-2 text-text-muted text-[11px]">"(letters, digits, ._- ; no slashes)"</span>
                        }.into_any()
                    }}
                </label>
                <div class="flex items-center gap-1">
                    <span class="text-text-muted text-xs font-mono">"target://"</span>
                    <input
                        type="text"
                        disabled=ref_locked
                        class=move || {
                            let base = "flex-1 px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm font-mono focus:outline-none focus:border-accent-amber";
                            if ref_locked { format!("{} opacity-60 cursor-not-allowed", base) } else { base.to_string() }
                        }
                        placeholder="mikrotik-edge"
                        prop:value=move || draft.get().ref_name
                        on:input=move |ev| draft.update(|d| d.ref_name = event_target_value(&ev))
                    />
                </div>
            </div>

            // Kind dropdown
            <div>
                <label class="block text-sm text-text-secondary mb-1">"Kind"</label>
                <select
                    class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                    on:change=move |ev| {
                        let v = event_target_value(&ev);
                        let new_kind = match v.as_str() {
                            "platform" => TargetKindDto::Platform,
                            "network" => TargetKindDto::Network,
                            "host" => TargetKindDto::Host,
                            "app" => TargetKindDto::App,
                            _ => return,
                        };
                        draft.update(|d| d.kind = new_kind);
                    }
                >
                    {TargetKindDto::all().iter().map(|k| {
                        let kind = *k;
                        view! {
                            <option
                                value=match kind {
                                    TargetKindDto::Platform => "platform",
                                    TargetKindDto::Network => "network",
                                    TargetKindDto::Host => "host",
                                    TargetKindDto::App => "app",
                                }
                                selected=move || draft.get().kind == kind
                            >
                                {kind.label()}
                            </option>
                        }
                    }).collect_view()}
                </select>
            </div>

            // Transport + port (side by side)
            <div class="grid grid-cols-2 gap-3">
                <div>
                    <label class="block text-sm text-text-secondary mb-1">"Transport"</label>
                    <select
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        on:change=move |ev| {
                            let v = event_target_value(&ev);
                            let new_t = match v.as_str() {
                                "ssh" => TransportKindDto::Ssh,
                                "rest" => TransportKindDto::Rest,
                                "snmp" => TransportKindDto::Snmp,
                                "grpc" => TransportKindDto::Grpc,
                                _ => return,
                            };
                            on_transport_change(new_t);
                        }
                    >
                        {TransportKindDto::all().iter().map(|t| {
                            let transport = *t;
                            view! {
                                <option
                                    value=match transport {
                                        TransportKindDto::Ssh => "ssh",
                                        TransportKindDto::Rest => "rest",
                                        TransportKindDto::Snmp => "snmp",
                                        TransportKindDto::Grpc => "grpc",
                                    }
                                    selected=move || draft.get().transport == transport
                                >
                                    {transport.label()}
                                </option>
                            }
                        }).collect_view()}
                    </select>
                </div>
                <div>
                    <label class="block text-sm text-text-secondary mb-1">"Port"</label>
                    <input
                        type="number"
                        min="1"
                        max="65535"
                        class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                        prop:value=move || draft.get().port
                        on:input=move |ev| draft.update(|d| {
                            d.port = event_target_value(&ev);
                            d.port_touched = true;
                        })
                    />
                </div>
            </div>

            // Host
            <div>
                <label class="block text-sm text-text-secondary mb-1">"Host"</label>
                <input
                    type="text"
                    class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm font-mono focus:outline-none focus:border-accent-amber"
                    placeholder="10.100.30.6 or pve01.lan"
                    prop:value=move || draft.get().host
                    on:input=move |ev| draft.update(|d| d.host = event_target_value(&ev))
                />
            </div>

            // Credential picker
            <div>
                <label class="block text-sm text-text-secondary mb-1">
                    "Credential"
                    <span class="ml-2 text-text-muted text-[11px]">"(from vault; manage at /admin/credentials)"</span>
                </label>
                <Suspense fallback=|| view! { <p class="text-text-muted text-xs">"Loading credentials..."</p> }>
                    {move || credentials_res.get().map(|result| match result {
                        Ok(creds) => view! {
                            <select
                                class="w-full px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                                on:change=move |ev| draft.update(|d| d.credential_ref = event_target_value(&ev))
                            >
                                <option value="" selected=move || draft.get().credential_ref.is_empty()>"— none —"</option>
                                {creds.into_iter().map(|c| {
                                    let value = format!("vault://{}", c.name);
                                    let value_for_selected = value.clone();
                                    let label = format!("{} ({})", c.name, c.kind.label());
                                    view! {
                                        <option
                                            value=value
                                            selected=move || draft.get().credential_ref == value_for_selected
                                        >
                                            {label}
                                        </option>
                                    }
                                }).collect_view()}
                            </select>
                        }.into_any(),
                        Err(e) => view! {
                            <p class="text-accent-danger text-xs">{format!("Failed to load credentials: {e}")}</p>
                        }.into_any(),
                    })}
                </Suspense>
            </div>

            // Labels (KV editor)
            <div>
                <label class="block text-sm text-text-secondary mb-1">"Labels"</label>
                <div class="space-y-2">
                    {move || {
                        let rows = draft.get().labels;
                        rows.into_iter().enumerate().map(|(idx, row)| {
                            view! {
                                <div class="flex gap-2">
                                    <input
                                        type="text"
                                        class="flex-1 px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                                        placeholder="key"
                                        prop:value=row.key.clone()
                                        on:input=move |ev| draft.update(|d| {
                                            if let Some(r) = d.labels.get_mut(idx) {
                                                r.key = event_target_value(&ev);
                                            }
                                        })
                                    />
                                    <input
                                        type="text"
                                        class="flex-1 px-3 py-2 bg-surface-tertiary border border-border-primary rounded-md text-text-primary text-sm focus:outline-none focus:border-accent-amber"
                                        placeholder="value"
                                        prop:value=row.value.clone()
                                        on:input=move |ev| draft.update(|d| {
                                            if let Some(r) = d.labels.get_mut(idx) {
                                                r.value = event_target_value(&ev);
                                            }
                                        })
                                    />
                                    <button
                                        type="button"
                                        on:click=move |_| draft.update(|d| { d.labels.remove(idx); })
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
                    on:click=add_label
                    class="text-xs text-accent-amber hover:underline mt-2"
                >
                    "+ Add Label"
                </button>
            </div>
        </div>
    }
}

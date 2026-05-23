//! Phase 2b #12 — server-fns backing `/admin/credentials`.
//!
//! Six thin admin-gate-then-forward wrappers over `Broker::vault_*`. Every
//! state-changing call is audited on the broker side (D23). D21 holds:
//! `daimon-app` does not import `daimon-vault` directly — wire DTOs defined
//! here mirror the broker types and convert only on the server side.

use leptos::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Wire kind discriminator. Serde rep matches `daimon_vault::CredentialKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKindDto {
    SshKey,
    SshPassword,
    ApiToken,
    Generic,
}

impl CredentialKindDto {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SshKey => "SSH Key",
            Self::SshPassword => "SSH Password",
            Self::ApiToken => "API Token",
            Self::Generic => "Generic",
        }
    }
}

/// Wire payload for credentials. Serde rep matches `daimon_vault::Credential`
/// so the JSON shape is identical end-to-end. No Zeroize on the DTO — the
/// real `Credential` is reconstructed server-side just before the broker call.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialDto {
    SshKey {
        username: String,
        private_key_pem: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        passphrase: Option<String>,
    },
    SshPassword {
        username: String,
        password: String,
    },
    ApiToken {
        token: String,
    },
    Generic {
        fields: BTreeMap<String, String>,
    },
}

impl CredentialDto {
    pub fn kind(&self) -> CredentialKindDto {
        match self {
            Self::SshKey { .. } => CredentialKindDto::SshKey,
            Self::SshPassword { .. } => CredentialKindDto::SshPassword,
            Self::ApiToken { .. } => CredentialKindDto::ApiToken,
            Self::Generic { .. } => CredentialKindDto::Generic,
        }
    }
}

/// List row — metadata only (no secret material).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRow {
    pub id: uuid::Uuid,
    pub name: String,
    pub kind: CredentialKindDto,
    pub created_at: String,
    pub updated_at: String,
}

// -------- Server-side bridge: DTO <-> broker types ---------------------------

#[cfg(feature = "ssr")]
mod bridge {
    use super::*;
    use daimon_broker::{Credential, CredentialKind, CredentialMetadata};

    impl From<CredentialKind> for CredentialKindDto {
        fn from(k: CredentialKind) -> Self {
            match k {
                CredentialKind::SshKey => Self::SshKey,
                CredentialKind::SshPassword => Self::SshPassword,
                CredentialKind::ApiToken => Self::ApiToken,
                CredentialKind::Generic => Self::Generic,
            }
        }
    }

    impl From<CredentialMetadata> for CredentialRow {
        fn from(m: CredentialMetadata) -> Self {
            Self {
                id: m.id,
                name: m.name,
                kind: m.kind.into(),
                created_at: m.created_at.to_rfc3339(),
                updated_at: m.updated_at.to_rfc3339(),
            }
        }
    }

    impl From<Credential> for CredentialDto {
        // Credential derives Drop (via ZeroizeOnDrop) so we can't move fields
        // out of it via destructuring. Clone-from-ref instead; the original
        // Credential drops at end of scope and triggers Zeroize on the source
        // bytes. Cloned values land in the (non-Zeroize) DTO for wire transit.
        fn from(c: Credential) -> Self {
            match &c {
                Credential::SshKey {
                    username,
                    private_key_pem,
                    passphrase,
                } => Self::SshKey {
                    username: username.clone(),
                    private_key_pem: private_key_pem.clone(),
                    passphrase: passphrase.clone(),
                },
                Credential::SshPassword { username, password } => Self::SshPassword {
                    username: username.clone(),
                    password: password.clone(),
                },
                Credential::ApiToken { token } => Self::ApiToken {
                    token: token.clone(),
                },
                Credential::Generic { fields } => Self::Generic {
                    fields: fields.clone(),
                },
            }
        }
    }

    impl From<CredentialDto> for Credential {
        fn from(d: CredentialDto) -> Self {
            match d {
                CredentialDto::SshKey {
                    username,
                    private_key_pem,
                    passphrase,
                } => Credential::SshKey {
                    username,
                    private_key_pem,
                    passphrase,
                },
                CredentialDto::SshPassword { username, password } => Credential::SshPassword {
                    username,
                    password,
                },
                CredentialDto::ApiToken { token } => Credential::ApiToken { token },
                CredentialDto::Generic { fields } => Credential::Generic { fields },
            }
        }
    }
}

// -------- Server-fns ---------------------------------------------------------

#[server]
pub async fn list_credentials() -> Result<Vec<CredentialRow>, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let rows = state
        .broker
        .vault_list_metadata(&claims.sub)
        .await
        .map_err(|e| ServerFnError::new(format!("vault_list_metadata: {e}")))?;
    Ok(rows.into_iter().map(CredentialRow::from).collect())
}

#[server]
pub async fn create_credential(
    name: String,
    cred: CredentialDto,
) -> Result<uuid::Uuid, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    state
        .broker
        .vault_create(&claims.sub, &name, cred.into())
        .await
        .map_err(|e| ServerFnError::new(format!("vault_create: {e}")))
}

#[server]
pub async fn update_credential(id: uuid::Uuid, cred: CredentialDto) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    state
        .broker
        .vault_update(&claims.sub, id, cred.into())
        .await
        .map_err(|e| ServerFnError::new(format!("vault_update: {e}")))
}

#[server]
pub async fn rename_credential(id: uuid::Uuid, new_name: String) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    state
        .broker
        .vault_rename(&claims.sub, id, &new_name)
        .await
        .map_err(|e| ServerFnError::new(format!("vault_rename: {e}")))
}

#[server]
pub async fn delete_credential(id: uuid::Uuid) -> Result<(), ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    state
        .broker
        .vault_delete(&claims.sub, id)
        .await
        .map_err(|e| ServerFnError::new(format!("vault_delete: {e}")))
}

#[server]
pub async fn reveal_credential(id: uuid::Uuid) -> Result<CredentialDto, ServerFnError> {
    use crate::auth_guard::require_admin;
    use crate::state::AppState;

    let claims = require_admin().await?;
    let state = expect_context::<AppState>();
    let cred = state
        .broker
        .vault_reveal(&claims.sub, id)
        .await
        .map_err(|e| ServerFnError::new(format!("vault_reveal: {e}")))?;
    Ok(cred.into())
}

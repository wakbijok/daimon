# daimon configuration reference

Resolution precedence for every key: **DB `app_config` (operator edit) → environment variable → compiled default** (FR-CFG-02). Bootstrap secrets (`DAIMON_PG_URL`, master key, `DAIMON_DATA_DIR`) are env/credential-sourced and never in `app_config` (FR-CFG-03).

## Consumed keys (read by the runtime)

| key | description |
|-----|-------------|
| `identity.org_name` | Organisation name shown in the console header |
| `llm.provider` | Active LLM provider: anthropic | openai | chatgpt | local |
| `llm.default_model.chat` | Model for the chat/worker role |
| `llm.available_models` | Comma-separated models an operator may pick in chat (unset = default only) |
| `llm.anthropic_key` | Anthropic API key (secret → vault ref) |
| `llm.openai_key` | OpenAI API key (secret → vault ref) |
| `llm.ollama_url` | Base URL for the local/Ollama provider |
| `guard.approval_timeout_secs` | Seconds to await an approval before denying |
| `guard.blast_radius_depth` | Graph traversal depth for the approval blast radius |
| `observer.prom_poll_interval_secs` | Prometheus poll interval (seconds) |
| `channels.telegram.enabled` | Enable the Telegram gateway |
| `channels.telegram.mode` | Telegram ingress mode: poll | webhook |
| `channels.telegram.bot_token_cred` | Telegram bot token (secret → vault ref) |
| `channels.telegram.webhook_secret_cred` | Telegram webhook signing secret credential (webhook mode only) |
| `channels.telegram.offset` | Telegram getUpdates offset (runtime cursor) |
| `channels.matrix.` | Matrix gateway configuration (enabled, homeserver, token cred, …) |
| `channels.alerts.` | Outbound alert routing rules (by class + severity → recipient) |
| `targets.` | Registered managed targets (target://<name>) + driver/connector binding |
| `chat.history_retention_days` | Days to retain chat transcripts (0 = forever); independent of the auth-session TTL |

## Read-only domains (not runtime-consumed in this build)

- `connections.*`
- `vault.*`

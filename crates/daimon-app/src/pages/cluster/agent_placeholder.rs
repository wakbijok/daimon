use leptos::prelude::*;

#[component]
pub fn AgentPlaceholder(
    tab_name: &'static str,
    description: &'static str,
) -> impl IntoView {
    view! {
        <div class="flex flex-col items-center justify-center gap-3 min-h-[200px] text-text-muted">
            <span style="font-size: 36px; line-height: 1;">"ℹ"</span>
            <p class="font-bold text-text-secondary" style="font-size: 14px;">
                {format!("Install daimon-agent for {} data", tab_name)}
            </p>
            <p class="text-center max-w-md" style="font-size: 12px;">
                {description}
            </p>
            <code class="px-3 py-1.5 rounded border border-border-primary bg-surface-tertiary text-accent-amber"
                style="font-size: 11px;">
                "curl -fsSL https://daimon.dev/install.sh | sh"
            </code>
        </div>
    }
}

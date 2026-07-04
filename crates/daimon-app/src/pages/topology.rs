//! UI-5 — `/topology`: the estate topology view.
//!
//! An honest, always-available topology built from the registered INVENTORY
//! (`list_targets`) — the daimon control plane at the hub, every managed target
//! as a spoke, grouped into class lanes (Network · Platform · Hosts · Apps). It
//! draws what daimon actually reaches; it does not invent dependency edges the
//! graph tier may not hold (that tier is optional and has no list-all API). The
//! render is a hand-rolled, dependency-free SVG — same discipline as the
//! sparkline: identical on SSR and hydrate, no JS chart lib (wasm size).

use leptos::prelude::*;

use crate::admin_targets::{list_targets, TargetKindDto, TargetRow};

// --- layout constants (SVG user units; the viewBox scales to the container) --
const VIEW_W: f64 = 920.0;
const HUB_X: f64 = 92.0;
const HUB_R: f64 = 36.0;
const LANE0_X: f64 = 300.0;
const LANE_GAP: f64 = 168.0;
const LANE_HEADER_Y: f64 = 30.0;
const NODE_TOP: f64 = 74.0;
const NODE_GAP: f64 = 62.0;
const NODE_W: f64 = 148.0;
const NODE_H: f64 = 44.0;
const MIN_H: f64 = 320.0;

/// The four inventory classes, in lane order, with a friendly label and a
/// diagram accent (hex — a diagram hue is semantic, kept off the app accent).
const LANES: [(TargetKindDto, &str, &str); 4] = [
    (TargetKindDto::Network, "Network", "#F59E0B"),
    (TargetKindDto::Platform, "Platform", "#A78BFA"),
    (TargetKindDto::Host, "Hosts", "#4CAF50"),
    (TargetKindDto::App, "Apps", "#38BDF8"),
];

#[derive(Clone)]
struct NodeBox {
    x: f64, // center
    y: f64, // center
    title: String,
    sub: String,
    color: &'static str,
}

#[derive(Clone)]
struct Edge {
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    color: &'static str,
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    } else {
        s.to_string()
    }
}

/// Pure layout: place the hub + target nodes and the hub→node edges, and return
/// the total SVG height. Kept pure (no view) so it is unit-testable.
fn layout(targets: &[TargetRow]) -> (Vec<NodeBox>, Vec<Edge>, f64) {
    let max_len = LANES
        .iter()
        .map(|(kind, _, _)| targets.iter().filter(|t| t.kind == *kind).count())
        .max()
        .unwrap_or(0);
    let height = (NODE_TOP + max_len as f64 * NODE_GAP + 20.0).max(MIN_H);
    let hub_y = height / 2.0;

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for (i, (kind, _, color)) in LANES.iter().enumerate() {
        let lane_x = LANE0_X + i as f64 * LANE_GAP;
        for (j, t) in targets.iter().filter(|t| t.kind == *kind).enumerate() {
            let cy = NODE_TOP + j as f64 * NODE_GAP;
            let transport = format!("{:?}", t.transport).to_lowercase();
            nodes.push(NodeBox {
                x: lane_x,
                y: cy,
                title: trunc(&t.ref_name, 17),
                sub: trunc(&format!("{}:{} · {}", t.host, t.port, transport), 22),
                color,
            });
            edges.push(Edge {
                x1: HUB_X + HUB_R,
                y1: hub_y,
                x2: lane_x - NODE_W / 2.0,
                y2: cy,
                color,
            });
        }
    }
    (nodes, edges, height)
}

#[component]
pub fn Topology() -> impl IntoView {
    let targets = Resource::new(|| (), |_| list_targets());

    view! {
        <div class="space-y-5">
            <div class="flex items-baseline gap-3">
                <h1 class="text-xl font-semibold text-text-primary">"Topology"</h1>
                <span class="text-xs font-mono text-text-muted">"control-plane reach across the estate"</span>
                <a href="/settings" class="ml-auto text-xs px-3 py-1.5 rounded-md border border-border-primary text-text-secondary hover:text-accent-amber hover:border-accent-amber transition-colors">
                    "+ Register target → Settings"
                </a>
            </div>

            // legend
            <div class="flex flex-wrap gap-4">
                {LANES.iter().map(|(_, label, color)| view! {
                    <span class="inline-flex items-center gap-1.5 text-[11px] text-text-secondary">
                        <span class="w-2.5 h-2.5 rounded-sm shrink-0" style=format!("background-color:{color}")></span>
                        {*label}
                    </span>
                }).collect_view()}
            </div>

            <Suspense fallback=|| view! { <div class="text-text-muted text-sm py-10 text-center">"loading topology…"</div> }>
                {move || targets.get().map(|res| match res {
                    Err(e) => view! { <div class="text-accent-danger text-sm">{format!("error: {e}")}</div> }.into_any(),
                    Ok(rows) if rows.is_empty() => view! {
                        <div class="text-text-muted text-sm py-16 text-center border border-dashed border-border-primary rounded-xl">
                            "No targets registered yet — nothing to map. Register one in Settings → Connectors & Targets."
                        </div>
                    }.into_any(),
                    Ok(rows) => {
                        let (nodes, edges, height) = layout(&rows);
                        let hub_y = height / 2.0;
                        let viewbox = format!("0 0 {VIEW_W} {height}");
                        view! {
                            <div class="rounded-xl border border-border-primary bg-surface-secondary p-2 overflow-x-auto">
                                <svg
                                    viewBox=viewbox
                                    width="100%"
                                    style=format!("min-width:640px;max-height:{height}px")
                                    preserveAspectRatio="xMidYMid meet"
                                    xmlns="http://www.w3.org/2000/svg"
                                >
                                    // ---- edges (behind everything) ----
                                    {edges.into_iter().map(|e| {
                                        let d = format!(
                                            "M {:.1},{:.1} C {:.1},{:.1} {:.1},{:.1} {:.1},{:.1}",
                                            e.x1, e.y1,
                                            (e.x1 + e.x2) / 2.0, e.y1,
                                            (e.x1 + e.x2) / 2.0, e.y2,
                                            e.x2, e.y2,
                                        );
                                        view! {
                                            <path d=d fill="none" stroke=e.color stroke-width="1.2" stroke-opacity="0.35" />
                                        }
                                    }).collect_view()}

                                    // ---- lane headers ----
                                    {LANES.iter().enumerate().map(|(i, (_, label, color))| {
                                        let lane_x = LANE0_X + i as f64 * LANE_GAP;
                                        view! {
                                            <text x=lane_x y=LANE_HEADER_Y text-anchor="middle"
                                                fill=*color font-size="12" font-weight="600"
                                                style="text-transform:uppercase;letter-spacing:0.08em">
                                                {*label}
                                            </text>
                                        }
                                    }).collect_view()}

                                    // ---- hub ----
                                    <circle cx=HUB_X cy=hub_y r=HUB_R fill="#161B22" stroke="#F59E0B" stroke-width="2" />
                                    <text x=HUB_X y=hub_y text-anchor="middle" dominant-baseline="central"
                                        fill="#F59E0B" font-size="13" font-weight="700">"daimon"</text>

                                    // ---- target nodes ----
                                    {nodes.into_iter().map(|n| {
                                        let rx = n.x - NODE_W / 2.0;
                                        let ry = n.y - NODE_H / 2.0;
                                        view! {
                                            <g>
                                                <rect x=rx y=ry width=NODE_W height=NODE_H rx="8"
                                                    fill="#0D1117" stroke=n.color stroke-width="1.4" stroke-opacity="0.7" />
                                                <text x=n.x y=n.y - 4.0 text-anchor="middle"
                                                    fill="#E6EDF3" font-size="12" font-weight="600">{n.title}</text>
                                                <text x=n.x y=n.y + 11.0 text-anchor="middle"
                                                    fill="#6B7280" font-size="9.5" font-family="ui-monospace,monospace">{n.sub}</text>
                                            </g>
                                        }
                                    }).collect_view()}
                                </svg>
                            </div>
                            <p class="text-[11px] text-text-muted">
                                "Built from the registered inventory: every target daimon is configured to reach, grouped by class. Add or edit targets in Settings → Connectors & Targets."
                            </p>
                        }.into_any()
                    }
                })}
            </Suspense>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin_targets::TransportKindDto;

    fn t(kind: TargetKindDto, name: &str) -> TargetRow {
        TargetRow {
            ref_name: name.into(),
            kind,
            transport: TransportKindDto::Ssh,
            host: "10.0.0.1".into(),
            port: 22,
            label_count: 0,
            capability_count: 0,
        }
    }

    #[test]
    fn empty_uses_min_height_and_no_nodes() {
        let (nodes, edges, h) = layout(&[]);
        assert!(nodes.is_empty());
        assert!(edges.is_empty());
        assert_eq!(h, MIN_H);
    }

    #[test]
    fn one_edge_per_target_and_lane_grouping() {
        let rows = vec![
            t(TargetKindDto::Network, "edge-fw"),
            t(TargetKindDto::Host, "host-a"),
            t(TargetKindDto::Host, "host-b"),
        ];
        let (nodes, edges, _h) = layout(&rows);
        assert_eq!(nodes.len(), 3);
        assert_eq!(edges.len(), 3);
        // Two hosts share a lane → stacked at distinct y.
        let hosts: Vec<_> = nodes.iter().filter(|n| n.color == "#4CAF50").collect();
        assert_eq!(hosts.len(), 2);
        assert_ne!(hosts[0].y, hosts[1].y);
        assert_eq!(hosts[0].x, hosts[1].x);
    }

    #[test]
    fn height_grows_with_largest_lane() {
        let rows: Vec<_> = (0..5).map(|i| t(TargetKindDto::App, &format!("app-{i}"))).collect();
        let (_, _, h) = layout(&rows);
        assert!(h > MIN_H);
    }
}

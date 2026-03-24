use leptos::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// Global counter for generating unique gradient IDs per component instance.
/// Atomic ensures correctness in both SSR (multi-threaded) and hydrate modes.
static SPARKLINE_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

/// Converts a slice of data points into an SVG polyline `points` attribute string.
///
/// Normalises values to fit within the given `width` x `height` viewBox, with
/// 1px padding on all sides so the stroke is never clipped at the edge.
///
/// - Empty slice  → returns `""`
/// - Single point → centres horizontally, centres vertically
/// - Many points  → spreads x evenly across [0, width], y mapped to [1, height-1]
pub fn points_to_polyline(data: &[f64], width: u32, height: u32) -> String {
    if data.is_empty() {
        return String::new();
    }

    let w = width as f64;
    let h = height as f64;
    let padding = 1.0_f64;

    // Find value range for y normalisation
    let min = data.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = data.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let range = max - min;

    let n = data.len();

    data.iter()
        .enumerate()
        .map(|(i, &v)| {
            // x: single point centres; multiple points span full width
            let x = if n == 1 {
                w / 2.0
            } else {
                (i as f64 / (n - 1) as f64) * w
            };

            // y: SVG y-axis is top-down, so higher values map to smaller y.
            // When all values are equal (range == 0) place them in the middle.
            let y = if range == 0.0 {
                h / 2.0
            } else {
                // Map v into [padding, height - padding]
                let inner_h = h - 2.0 * padding;
                padding + (1.0 - (v - min) / range) * inner_h
            };

            format!("{:.1},{:.1}", x, y)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Inline SVG sparkline component.
///
/// Renders a small line chart as a plain SVG element — no JS, no canvas.
/// Works identically in SSR and hydrate modes.
///
/// For empty `data`, renders a text dash ("—") instead.
/// When `fill` is true a gradient-filled area is drawn beneath the stroke line.
#[component]
pub fn Sparkline(
    data: Vec<f64>,
    #[prop(default = String::from("#F59E0B"))] color: String,
    #[prop(default = 80)] width: u32,
    #[prop(default = 20)] height: u32,
    #[prop(default = true)] fill: bool,
) -> impl IntoView {
    if data.is_empty() {
        return view! {
            <span class="text-text-muted text-sm">"—"</span>
        }
        .into_any();
    }

    // Unique gradient ID so multiple instances on the same page don't collide.
    let id = SPARKLINE_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let gradient_id = format!("sg{}", id);

    let w = width as f64;
    let h = height as f64;
    let viewbox = format!("0 0 {} {}", w, h);
    let points = points_to_polyline(&data, width, height);

    // Fill polygon closes back along the bottom edge of the viewBox.
    // We append two extra points: bottom-right and bottom-left.
    let fill_points = if fill && !points.is_empty() {
        // Last x coordinate from the last data point
        let last_x = if data.len() == 1 {
            w / 2.0
        } else {
            w
        };
        format!("{} {},{} 0,{}", points, last_x, h, h)
    } else {
        String::new()
    };

    let grad_url = format!("url(#{gradient_id})");
    let color_clone = color.clone();

    view! {
        <svg
            width=width
            height=height
            viewBox=viewbox
            xmlns="http://www.w3.org/2000/svg"
            style="display:inline-block;vertical-align:middle;overflow:visible"
        >
            {if fill {
                view! {
                    <defs>
                        <linearGradient id=gradient_id x1="0" y1="0" x2="0" y2="1">
                            <stop offset="0%" stop-color=color_clone.clone() stop-opacity="0.3" />
                            <stop offset="100%" stop-color=color_clone.clone() stop-opacity="0.0" />
                        </linearGradient>
                    </defs>
                    <polyline
                        points=fill_points
                        fill=grad_url
                        stroke="none"
                    />
                    <polyline
                        points=points
                        fill="none"
                        stroke=color
                        stroke-width="1.5"
                        stroke-linejoin="round"
                        stroke-linecap="round"
                    />
                }.into_any()
            } else {
                view! {
                    <polyline
                        points=points
                        fill="none"
                        stroke=color
                        stroke-width="1.5"
                        stroke-linejoin="round"
                        stroke-linecap="round"
                    />
                }.into_any()
            }}
        </svg>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// Unit tests — pure function only; no Leptos runtime required
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_data_returns_empty_string() {
        assert_eq!(points_to_polyline(&[], 80, 20), "");
    }

    #[test]
    fn single_point_centered() {
        let result = points_to_polyline(&[5.0], 80, 20);
        assert!(result.contains("40.0")); // centered x
    }

    #[test]
    fn two_points_span_width() {
        let result = points_to_polyline(&[0.0, 10.0], 100, 20);
        let parts: Vec<&str> = result.split(' ').collect();
        assert_eq!(parts.len(), 2);
        assert!(parts[0].starts_with("0.0")); // x=0
        assert!(parts[1].starts_with("100.0")); // x=100
    }
}

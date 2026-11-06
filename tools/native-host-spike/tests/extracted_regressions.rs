//! Host geometry regressions stay with the view, not the VT core.
use native_host_spike::{CssRect, Policy, Terminal, map_css_rect};
#[test]
fn dpi_rect_and_visibility_gate_input_without_disabling_business_webview() {
    assert_eq!(
        map_css_rect(
            CssRect {
                x: 20.,
                y: 64.,
                width: 300.,
                height: 200.
            },
            1.5,
            1
        ),
        Some((30, 96, 450, 300))
    );
    for (hidden, modal) in [(true, false), (false, true)] {
        let p = Policy { hidden, modal };
        assert!(!p.native_visible() && !p.native_accepts_input());
    }
}
#[test]
fn invalid_or_empty_dom_rect_never_accepts_input() {
    for width in [0., -1., f64::NAN] {
        assert!(
            map_css_rect(
                CssRect {
                    x: 0.,
                    y: 0.,
                    width,
                    height: 100.
                },
                1.,
                1
            )
            .is_none()
        );
    }
}
#[test]
fn unicode_wide_combining_and_proportional_fallback_cannot_shift_columns() {
    let mut t = Terminal::new(20, 4);
    t.feed("Wi中e\u{301}🙂Z\r\nאבX".as_bytes());
    let g = t.snapshot();
    let placements: Vec<_> = g
        .cells
        .iter()
        .enumerate()
        .filter(|(_, c)| c.width > 0 && c.text != " ")
        .map(|(i, c)| {
            (
                i,
                12. + (i % g.cols) as f32 * 11.,
                12. + (i / g.cols) as f32 * 23.,
                c.width as f32 * 11.,
            )
        })
        .collect();
    let first: Vec<_> = placements
        .iter()
        .filter(|p| p.2 == 12.)
        .map(|p| (p.0, p.1, p.3))
        .collect();
    assert_eq!(
        first,
        [
            (0, 12., 11.),
            (1, 23., 11.),
            (2, 34., 22.),
            (4, 56., 11.),
            (5, 67., 22.),
            (7, 89., 11.)
        ]
    );
    assert_eq!(
        placements
            .iter()
            .filter(|p| p.2 == 35.)
            .map(|p| p.1)
            .collect::<Vec<_>>(),
        [12., 23., 34.]
    );
    assert_eq!(g.cells[4].text, "e\u{301}");
}
#[test]
fn resize_matches_physical_grid_and_clamps_minimum() {
    let grid = native_host_spike::terminal_session::grid_size;
    assert_eq!(grid(1060, 560), (94, 23));
    assert_eq!(grid(904, 484), (80, 20));
    assert_eq!(grid(0, 0), (2, 1));
}

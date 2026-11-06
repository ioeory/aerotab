use native_host_spike::{CssRect, map_css_rect};

#[test]
fn css_rect_maps_to_gtk_logical_using_both_scales() {
    let rect = CssRect {
        x: 10.0,
        y: 20.0,
        width: 200.0,
        height: 100.0,
    };
    assert_eq!(map_css_rect(rect, 2.0, 2), Some((10, 20, 200, 100)));
    assert_eq!(map_css_rect(rect, 1.5, 1), Some((15, 30, 300, 150)));
    assert_eq!(map_css_rect(rect, 2.0, 1), Some((20, 40, 400, 200)));
}

#[test]
fn rejects_invalid_or_unbounded_geometry() {
    let rect = CssRect {
        x: 0.0,
        y: 0.0,
        width: f64::NAN,
        height: 10.0,
    };
    assert_eq!(map_css_rect(rect, 1.0, 1), None);
    assert_eq!(
        map_css_rect(
            CssRect {
                width: 10.0,
                ..rect
            },
            0.0,
            1
        ),
        None
    );
}

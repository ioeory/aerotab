#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Debug, PartialEq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub visible: bool,
    pub input: bool,
}
pub fn place(rect: Rect, scale: f64, active: bool, modal: bool) -> Placement {
    let valid = [rect.x, rect.y, rect.width, rect.height, scale]
        .iter()
        .all(|v| v.is_finite())
        && rect.width > 0.0
        && rect.height > 0.0
        && scale > 0.0;
    let visible = valid && active && !modal;
    Placement {
        x: (rect.x * scale).round() as i32,
        y: (rect.y * scale).round() as i32,
        width: (rect.width * scale).round().max(0.0) as i32,
        height: (rect.height * scale).round().max(0.0) as i32,
        visible,
        input: visible,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dpi_rect_and_visibility_gate_input_without_disabling_business_webview() {
        let rect = Rect {
            x: 20.0,
            y: 64.0,
            width: 300.0,
            height: 200.0,
        };
        assert_eq!(
            place(rect, 1.5, true, false),
            Placement {
                x: 30,
                y: 96,
                width: 450,
                height: 300,
                visible: true,
                input: true
            }
        );
        for (active, modal) in [(false, false), (true, true)] {
            let p = place(rect, 1.0, active, modal);
            assert!(!p.visible && !p.input);
        }
    }
    #[test]
    fn invalid_or_empty_dom_rect_never_accepts_input() {
        for width in [0.0, -1.0, f64::NAN] {
            assert!(
                !place(
                    Rect {
                        x: 0.0,
                        y: 0.0,
                        width,
                        height: 100.0
                    },
                    1.0,
                    true,
                    false
                )
                .visible
            );
        }
    }
}

use serde::Serialize;
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<&'static str>,
    pub geometry_unit: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}
/// Kept for call-site compatibility; Windows no longer requires debug E2E opt-in.
pub fn test_policy(enabled: bool, os: &str, _debug: bool, _opted_in: bool) -> Capabilities {
    policy(enabled, os)
}
pub fn policy(enabled: bool, os: &str) -> Capabilities {
    let available = enabled && matches!(os, "linux" | "windows");
    Capabilities {
        available,
        backend: match (available, os) {
            (true, "windows") => Some("win32-dx12-glyphon"),
            (true, _) => Some("gtk-glarea"),
            _ => None,
        },
        geometry_unit: "css-px",
        reason: if !enabled {
            Some("native-view feature is disabled")
        } else if os == "macos" {
            Some("AppKit/Metal pointer/selection, preedit and configurable shortcut routing are implemented but unverified on macOS; full Apple-SDK workspace compilation, native input and bidirectional handoff acceptance are pending")
        } else if !available {
            Some("native-view is not implemented on this platform")
        } else {
            None
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_follows_feature_flag_without_debug_gates() {
        for enabled in [false, true] {
            for debug in [false, true] {
                for opted_in in [false, true] {
                    let c = test_policy(enabled, "windows", debug, opted_in);
                    assert_eq!(c.available, enabled);
                    if c.available {
                        assert_eq!(c.backend, Some("win32-dx12-glyphon"));
                        assert!(c.reason.is_none());
                    }
                    assert!(!test_policy(enabled, "macos", debug, opted_in).available);
                }
            }
        }
        assert!(policy(true, "windows").available);
    }
    #[test]
    fn macos_is_not_advertised_before_native_acceptance() {
        let c = policy(true, "macos");
        assert!(!c.available);
        assert!(c.reason.unwrap().contains("AppKit/Metal"));
        assert!(c.reason.unwrap().contains("pending"));
    }
    #[test]
    fn feature_and_platform_policy() {
        for enabled in [false, true] {
            for os in ["linux", "windows", "macos"] {
                let c = policy(enabled, os);
                assert_eq!(c.available, enabled && matches!(os, "linux" | "windows"));
                assert_eq!(c.geometry_unit, "css-px");
                assert_eq!(c.reason.is_none(), c.available);
            }
        }
    }
}

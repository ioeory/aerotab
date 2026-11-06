//! Opt-in native views of existing terminal engines; never owns a session/PTY.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
#[cfg(any(target_os = "windows", target_os = "macos", test))]
mod font_family;
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos", test))]
mod input;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "macos", test))]
mod macos_policy;
#[cfg(target_os = "macos")]
mod macos_renderer;
#[cfg(target_os = "linux")]
mod renderer;
mod shortcut;
mod snapshot;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(any(target_os = "windows", test))]
mod windows_glyph_cache;
#[cfg(target_os = "windows")]
mod windows_renderer;
pub use snapshot::{Cell, Snapshot};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostError {
    pub code: String,
    pub message: String,
}
impl HostError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Geometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Default for Geometry {
    fn default() -> Self {
        Self {
            x: 0.,
            y: 0.,
            width: 640.,
            height: 480.,
        }
    }
}
impl Geometry {
    pub fn validate(self) -> Result<Self, HostError> {
        if [self.x, self.y, self.width, self.height]
            .iter()
            .any(|v| !v.is_finite() || *v < 0. || *v > 16384.)
            || self.width < 1.
            || self.height < 1.
        {
            return Err(HostError::new(
                "invalid_geometry",
                "CSS bounds must be finite, positive and <=16384",
            ));
        }
        Ok(self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[serde(default)]
pub struct ViewConfig {
    pub font_family: String,
    pub font_size: f64,
    pub font_weight: u16,
    pub line_padding: f64,
    pub foreground: [u8; 3],
    pub background: [u8; 3],
    pub palette: Vec<[u8; 3]>,
    /// Attachment epoch required on every update, including geometry-only updates.
    pub expected_generation: Option<Uuid>,
    pub revision: u32,
    pub input_generation: Option<Uuid>,
    pub input_ready: bool,
    pub input_frozen: bool,
    pub input_ack: u32,
}
impl Default for ViewConfig {
    fn default() -> Self {
        Self {
            expected_generation: None,
            revision: 0,
            input_generation: None,
            input_ready: false,
            input_frozen: false,
            input_ack: 0,
            font_family: "monospace".into(),
            font_size: 17.,
            font_weight: 400,
            line_padding: 3.,
            foreground: [226, 237, 247],
            background: [24, 28, 36],
            palette: (0..16)
                .map(|i| {
                    let c = super::engine::default_rgb(i);
                    [c.r, c.g, c.b]
                })
                .collect(),
        }
    }
}
/// CSS allocation excludes the host's 12px padding on each edge.
pub fn grid_size(width: i32, height: i32, cw: u16, ch: u16) -> (u16, u16) {
    (
        ((width - 24).clamp(1, 16384) as u16 / cw.max(1)).max(1),
        ((height - 24).clamp(1, 16384) as u16 / ch.max(1)).max(1),
    )
}
impl ViewConfig {
    pub fn validate(&self) -> Result<(), HostError> {
        if self.font_family.trim().is_empty()
            || self.font_family.len() > 512
            || !self.font_size.is_finite()
            || !(8. ..=32.).contains(&self.font_size)
            || !self.line_padding.is_finite()
            || !(0. ..=16.).contains(&self.line_padding)
            || !(100..=900).contains(&self.font_weight)
        {
            return Err(HostError::new("invalid_config", "invalid font settings"));
        }
        if self.palette.len() != 16 {
            Err(HostError::new(
                "invalid_config",
                "palette requires exactly 16 colors",
            ))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewState {
    pub session_id: Uuid,
    pub generation: Uuid,
    pub geometry: Geometry,
    pub visible: bool,
    pub last_draw_seq: Option<u32>,
    pub revision: u32,
    pub drawn_revision: Option<u32>,
    /// Decimal u64 strings; never JSON numbers or wrapping wire sequence.
    pub handoff_output_seq: String,
    pub drawn_output_seq: Option<String>,
    pub input_accepted: u32,
    pub input_ack: u32,
    pub input_frozen: bool,
    pub error: Option<HostError>,
    pub backend: &'static str,
}
#[derive(Default)]
pub struct Registry {
    views: HashMap<Uuid, ViewState>,
}
impl Registry {
    pub fn attach(
        &mut self,
        id: Uuid,
        geometry: Geometry,
        cfg: ViewConfig,
    ) -> Result<ViewState, HostError> {
        geometry.validate()?;
        cfg.validate()?;
        if self.views.contains_key(&id) {
            return Err(HostError::new("already_attached", "detach old view first"));
        }
        let state = ViewState {
            session_id: id,
            generation: Uuid::new_v4(),
            geometry,
            visible: true,
            last_draw_seq: None,
            revision: 0,
            drawn_revision: None,
            handoff_output_seq: "0".into(),
            drawn_output_seq: None,
            input_accepted: 0,
            input_ack: 0,
            input_frozen: false,
            error: None,
            backend: "gtk-glarea",
        };
        self.views.insert(id, state.clone());
        Ok(state)
    }
    pub fn get(&self, id: Uuid) -> Result<ViewState, HostError> {
        self.views
            .get(&id)
            .cloned()
            .ok_or_else(|| HostError::new("not_attached", "native view is not attached"))
    }
    pub fn update(
        &mut self,
        id: Uuid,
        geometry: Geometry,
        visible: bool,
    ) -> Result<ViewState, HostError> {
        geometry.validate()?;
        let v = self
            .views
            .get_mut(&id)
            .ok_or_else(|| HostError::new("not_attached", "native view is not attached"))?;
        v.geometry = geometry;
        v.visible = visible;
        Ok(v.clone())
    }
    pub fn detach(&mut self, id: Uuid) -> Result<(), HostError> {
        self.views
            .remove(&id)
            .ok_or_else(|| HostError::new("not_attached", "native view is not attached"))?;
        Ok(())
    }
}
pub type InputDispatch = std::sync::Arc<dyn Fn(&'static str, serde_json::Value) + Send + Sync>;
/// All operations must execute on the Tauri main thread; GTK values remain thread-local.
pub struct NativeHost;
impl NativeHost {
    #[cfg(all(debug_assertions, target_os = "windows"))]
    pub fn debug_windows_state(id: Uuid) -> Result<serde_json::Value, HostError> {
        windows::debug_state(id)
    }
    #[cfg(all(debug_assertions, target_os = "windows"))]
    pub fn debug_windows_messages(
        id: Uuid,
        generation: Uuid,
        text: &str,
    ) -> Result<ViewState, HostError> {
        windows::debug_messages(id, generation, text)
    }
    pub fn attach(
        window: &tauri::WebviewWindow,
        id: Uuid,
        geometry: Geometry,
        cfg: ViewConfig,
        engine: std::sync::Arc<super::engine::TerminalEngine>,
        dispatch: InputDispatch,
    ) -> Result<ViewState, HostError> {
        geometry.validate()?;
        cfg.validate()?;
        #[cfg(target_os = "linux")]
        {
            linux::attach(window, id, geometry, cfg, engine, dispatch)
        }
        #[cfg(target_os = "windows")]
        {
            windows::attach(window, id, geometry, cfg, engine, dispatch)
        }
        #[cfg(target_os = "macos")]
        {
            macos::attach(window, id, geometry, cfg, engine, dispatch)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            let _ = (window, id, geometry, cfg, engine, dispatch);
            Err(HostError::new(
                "unsupported_platform",
                "native-view is implemented on Linux only",
            ))
        }
    }
    pub fn focus(id: Uuid) -> Result<(), HostError> {
        #[cfg(target_os = "linux")]
        {
            linux::focus(id)
        }
        #[cfg(target_os = "windows")]
        {
            windows::focus(id)
        }
        #[cfg(target_os = "macos")]
        {
            macos::focus(id)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            let _ = id;
            Err(HostError::new(
                "unsupported_platform",
                "native-view unsupported",
            ))
        }
    }
    pub fn update(
        id: Uuid,
        geometry: Geometry,
        visible: bool,
        cfg: Option<ViewConfig>,
    ) -> Result<ViewState, HostError> {
        if let Some(cfg) = &cfg {
            cfg.validate()?;
        }
        geometry.validate()?;
        #[cfg(target_os = "linux")]
        {
            linux::update(id, geometry, visible, cfg)
        }
        #[cfg(target_os = "windows")]
        {
            windows::update(id, geometry, visible, cfg)
        }
        #[cfg(target_os = "macos")]
        {
            macos::update(id, geometry, visible, cfg)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            let _ = (id, visible, cfg);
            Err(HostError::new(
                "unsupported_platform",
                "native-view unsupported",
            ))
        }
    }
    pub fn detach(id: Uuid) -> Result<(), HostError> {
        #[cfg(target_os = "linux")]
        {
            linux::detach(id)
        }
        #[cfg(target_os = "windows")]
        {
            windows::detach(id)
        }
        #[cfg(target_os = "macos")]
        {
            macos::detach(id)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            let _ = id;
            Err(HostError::new(
                "unsupported_platform",
                "native-view unsupported",
            ))
        }
    }
    pub fn state(id: Uuid) -> Result<ViewState, HostError> {
        #[cfg(target_os = "linux")]
        {
            linux::state(id)
        }
        #[cfg(target_os = "windows")]
        {
            windows::state(id)
        }
        #[cfg(target_os = "macos")]
        {
            macos::state(id)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
        {
            let _ = id;
            Err(HostError::new(
                "unsupported_platform",
                "native-view unsupported",
            ))
        }
    }
}
#[cfg(test)]
mod tests;
#[cfg(any(target_os = "windows", test))]
mod windows_policy;

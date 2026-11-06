#[allow(dead_code)]
#[path = "../main.rs"]
mod composition;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            // This is a real Tauri runtime-owned native window. No foreign surface reparenting.
            let window = tauri::WindowBuilder::new(app, "native-host")
                .title("Tauri native-host composition spike")
                .inner_size(1100.0, 680.0)
                .build()?;
            let gtk_window = window.gtk_window()?;
            let vbox = window.default_vbox()?;
            use gtk::prelude::*;
            composition::compose(gtk_window.upcast(), Some(vbox));
            Ok(())
        })
        .run(tauri::generate_context!("tauri.conf.json"))
        .expect("isolated Tauri host runtime");
}

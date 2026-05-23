fn main() {
    #[cfg(feature = "desktop")]
    {
        println!("cargo:rerun-if-changed=tauri.conf.json");
        println!("cargo:rerun-if-changed=../apps/ui/dist");
        println!("cargo:rerun-if-changed=../apps/ui/src");
        println!("cargo:rerun-if-changed=../apps/ui/package.json");
        println!("cargo:rerun-if-changed=../apps/ui/vite.config.ts");
    }
    #[cfg(feature = "desktop")]
    tauri_build::build();
}

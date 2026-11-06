fn main() {
    println!("cargo:rerun-if-changed=dist/index.html");
    println!("cargo:rerun-if-changed=tauri.conf.json");
}

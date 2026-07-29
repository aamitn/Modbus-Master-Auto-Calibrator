// build.rs
fn main() {
    // Only execute this build script when compiling for Windows
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icon.ico"); // Path to your icon file
        res.compile().unwrap();
    }
}
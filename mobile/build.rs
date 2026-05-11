fn main() {
    // tauri-build doesn't automatically rerun when `dist/` content
    // changes, so edits to the loading-screen HTML / JS can land in a
    // stale binary if the rest of the crate didn't change. Surface
    // every file under `dist/` so cargo rebuilds whenever any of them
    // does (SPEC-040 footgun-prevention).
    println!("cargo:rerun-if-changed=dist");

    tauri_build::build()
}

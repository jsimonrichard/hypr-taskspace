//! Records a bundled Waybar cdylib path when built with Cargo artifact dependencies.
//!
//! Stable builds: run `cargo build -p tsk-waybar --release` before `scripts/install-user-share.sh`,
//! or let install build the module from the workspace.
//!
//! Nightly artifact-deps build (optional):
//! ```toml
//! [build-dependencies]
//! tsk-waybar = { path = "../tsk-waybar", artifact = "cdylib" }
//! ```
//! Then: `cargo build -Z bindeps -p tsk-cli`

fn main() {
    println!("cargo:rerun-if-changed=../tsk-waybar");
    if let Ok(so) = std::env::var("CARGO_CDYLIB_FILE_TSK_WAYBAR") {
        println!("cargo:rustc-env=TSK_WAYBAR_CDYLIB_SOURCE={so}");
    }
}

// build.rs — kernel pre-build script
//
// Guarantees that fs.img exists at the workspace root before `cargo build`
// processes `include_bytes!("../../fs.img")` in main.rs.
//
// If the image has already been built by tools/pack-fs.sh it is left
// untouched.  If it is missing (fresh checkout, clean tree) an empty but
// valid WasmFS image is written: one 512-byte directory block of all zeros,
// which mount_from_image will read as "no files".

use std::path::PathBuf;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let img_path: PathBuf = PathBuf::from(&manifest)
        .parent()
        .unwrap()
        .join("fs.img");

    if !img_path.exists() {
        // Empty WasmFS image: one zeroed directory block.
        std::fs::write(&img_path, vec![0u8; 512]).expect("failed to create empty fs.img");
        eprintln!("build.rs: created empty fs.img at {}", img_path.display());
    }

    // Re-run this script whenever fs.img changes so the kernel re-embeds it.
    println!("cargo:rerun-if-changed={}", img_path.display());
}

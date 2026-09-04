use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rustc-check-cfg=cfg(feature, values(\"experimental-1x-dsp\"))");
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let path = root.join("src/oscillators/va/antialias.rs");
    let s = fs::read_to_string(&path).unwrap().replace(
        "use truce_simd::simd::{f32x4, f32x8};",
        "use wide::{f32x4, f32x8};",
    );
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("antialias.rs"),
        s,
    )
    .unwrap();
    println!("cargo:rerun-if-changed={}", path.display());
}

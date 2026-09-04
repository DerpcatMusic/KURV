use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rustc-check-cfg=cfg(feature, values(\"experimental-1x-dsp\"))");
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let p = root.join("src/oscillators/va/antialias.rs");
    fs::write(
        out.join("baseline.rs"),
        fs::read_to_string(&p).unwrap().replace(
            "use truce_simd::simd::{f32x4, f32x8};",
            "use wide::{f32x4, f32x8};",
        ),
    )
    .unwrap();
    println!("cargo:rerun-if-changed={}", p.display());
}

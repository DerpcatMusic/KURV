use std::{env, fs, path::PathBuf};
fn main() {
    let root = env::var_os("KURV_SOURCE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../..")
        });
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    for name in ["antialias", "pm_quality"] {
        let p = root.join(format!("src/oscillators/va/{name}.rs"));
        let s = fs::read_to_string(&p).unwrap().replace(
            "use truce_simd::simd::{f32x4, f32x8};",
            "use wide::{f32x4, f32x8};",
        );
        let s = s
            .replace("use truce_simd::simd::f32x8;", "use wide::f32x8;")
            .replace("//!", "//");
        fs::write(out.join(format!("{name}.rs")), s).unwrap();
        println!("cargo:rerun-if-changed={}", p.display());
    }
    println!("cargo:rerun-if-env-changed=KURV_SOURCE_ROOT");
}

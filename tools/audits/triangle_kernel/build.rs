use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let path = "src/oscillators/va/antialias.rs";
    let revision = env::var("KURV_AUDIT_BASE")
        .unwrap_or_else(|_| "d084681411a95803bb52206647c2bc881c4cbf8b".into());
    let baseline = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["show", &format!("{revision}:{path}")])
        .output()
        .expect("git is required");
    assert!(
        baseline.status.success(),
        "baseline revision must exist locally"
    );
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    for (name, source) in [
        ("before", String::from_utf8(baseline.stdout).unwrap()),
        ("after", fs::read_to_string(root.join(path)).unwrap()),
    ] {
        // truce-simd reexports wide's f32x4/f32x8. Isolate this production module
        // from plugin/UI/licensing dependencies; do not reimplement its arithmetic.
        let source = source.replace(
            "use truce_simd::simd::{f32x4, f32x8};",
            "use wide::{f32x4, f32x8};",
        );
        fs::write(out.join(format!("{name}.rs")), source).unwrap();
    }
    println!("cargo:rerun-if-changed={}", root.join(path).display());
    println!("cargo:rerun-if-env-changed=KURV_AUDIT_BASE");
}

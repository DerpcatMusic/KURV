use std::{env, fs, path::PathBuf};

fn function<'a>(source: &'a str, name: &str) -> &'a str {
    let start = source
        .find(&format!("pub fn {name}"))
        .expect("production function exists");
    let body = source[start..].find('{').unwrap() + start;
    let mut depth = 0;
    for (offset, byte) in source.as_bytes()[body..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &source[start..=body + offset];
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced production function");
}

fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let antialias = root.join("src/oscillators/va/antialias.rs");
    let render = root.join("src/oscillators/va/render.rs");
    // truce-simd 6.3.0 reexports these exact wide types.
    let aa = fs::read_to_string(&antialias).unwrap().replace(
        "use truce_simd::simd::{f32x4, f32x8};",
        "use wide::{f32x4, f32x8};",
    );
    fs::write(out.join("antialias.rs"), aa).unwrap();
    let render_source = fs::read_to_string(&render).unwrap();
    let functions = [
        "accumulate_spline_saw8_phase_modulated_block",
        "accumulate_spline_saw4_phase_modulated_block",
        "accumulate_spline_saw8_phase_modulated_lanes_block",
    ];
    let extracted = functions
        .map(|name| function(&render_source, name))
        .join("\n");
    fs::write(out.join("render.rs"), extracted).unwrap();
    println!("cargo:rerun-if-changed={}", antialias.display());
    println!("cargo:rerun-if-changed={}", render.display());
}

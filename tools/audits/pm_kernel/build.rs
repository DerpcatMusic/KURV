use std::{env, fs, path::PathBuf};

fn function<'a>(source: &'a str, name: &str) -> &'a str {
    let mut start = source
        .find(&format!("fn {name}"))
        .expect("production function exists");
    start = source[..start].rfind('\n').map_or(0, |n| n + 1);
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
    for variant in ["baseline", "candidate"] {
        let read = |path: &str| {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
            if variant == "candidate" {
                fs::read_to_string(root.join(path)).unwrap()
            } else {
                let result = std::process::Command::new("git")
                    .args([
                        "show",
                        &format!("d084681411a95803bb52206647c2bc881c4cbf8b:{path}"),
                    ])
                    .current_dir(&root)
                    .output()
                    .unwrap();
                assert!(result.status.success());
                String::from_utf8(result.stdout).unwrap()
            }
        };
        let aa = read("src/oscillators/va/antialias.rs").replace(
            "use truce_simd::simd::{f32x4, f32x8};",
            "use wide::{f32x4,f32x8};",
        );
        fs::write(out.join(format!("{variant}_aa.rs")), aa).unwrap();
        let source = read("src/oscillators/va/render.rs");
        let code = [
            "accumulate_spline_saw8_phase_modulated_block",
            "accumulate_spline_saw4_phase_modulated_block",
            "accumulate_spline_saw8_phase_modulated_lanes_block",
        ]
        .map(|name| function(&source, name))
        .join("\n");
        fs::write(out.join(format!("{variant}_render.rs")), code).unwrap();
        if variant == "candidate" {
            let source = read("src/oscillators/va/backend.rs");
            let mut code = function(&source, "accumulate_saw8_phase_modulated").to_string();
            code.push_str(function(&source, "accumulate_saw8_phase_modulated_lanes"));
            for name in [
                "accumulate_saw8_phase_modulated_avx2",
                "accumulate_saw8_phase_modulated_lanes_avx2",
                "spline_blep_residual_avx2",
            ] {
                code.push_str(
                    r#"
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(unsafe_op_in_unsafe_fn)]
"#,
                );
                code.push_str(function(&source, name));
            }
            fs::write(out.join("backend.rs"), code).unwrap();
        }
    }
}

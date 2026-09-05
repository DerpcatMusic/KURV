use std::{env, fs, path::Path};

// Keep every production DSP type and function. Only host preset-serialization
// adapters are omitted, so this executable needs no private auth or GUI deps.
fn prepare(path: &Path, output: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());
    let mut source = fs::read_to_string(path).unwrap();
    source = source.replace("use truce::State;", "");
    source = source.replace(
        "use truce_core::custom_state::{PersistField, StateCursor, StateField};",
        "",
    );
    source = source.replace(", State)]", ")]");
    while let Some(start) = source.find("impl PersistField for ") {
        let open = start + source[start..].find('{').unwrap();
        let mut depth = 0;
        let mut end = open;
        for (offset, c) in source[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                end = open + offset + 1;
                break;
            }
        }
        source.replace_range(start..end, "");
    }
    // The Rich resynthesis timeline depends on a different engine. Keep VA state
    // layout, but omit this method and its import from this VA-only harness.
    if path.ends_with("oscillators/va/mod.rs") {
        source = source.replace("use super::resynth::{GrainDirection, ResynthControls};", "");
        let start = source
            .find("    #[inline]\n    pub(crate) fn advance_rich_timeline(")
            .expect("Rich timeline marker changed");
        let end = start
            + source[start..]
                .find("    #[inline]\n    pub(crate) fn restart_rich_timeline")
                .expect("Rich method boundary changed");
        source.replace_range(start..end, "");
    }
    // These two tests require the serialization derives intentionally omitted
    // above. All other table tests and all oversampling/DSP tests remain enabled.
    if path.ends_with("oscillators/va/table.rs") {
        source = source.replace("    use truce_core::custom_state::State as StateTrait;", "");
        for name in [
            "appended_positions_field_preserves_legacy_state_and_round_trips_positioned_state",
            "function_frame_persists_and_reaches_audio_publication",
        ] {
            let marker = format!("    #[test]\n    fn {name}(");
            assert!(
                source.contains(&marker),
                "serialization test marker changed: {name}"
            );
            source = source.replace(
                &marker,
                &format!("    #[cfg(any())]\n    #[test]\n    fn {name}("),
            );
        }
    }
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    fs::write(output, source).unwrap();
}

fn tree(input: &Path, output: &Path) {
    println!("cargo:rerun-if-changed={}", input.display());
    for entry in fs::read_dir(input).unwrap() {
        let path = entry.unwrap().path();
        let target = output.join(path.file_name().unwrap());
        if path.is_dir() {
            tree(&path, &target);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            prepare(&path, &target);
        }
    }
}

fn main() {
    let manifest = std::path::PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.join("../../..");
    let output = std::path::PathBuf::from(env::var_os("OUT_DIR").unwrap());
    // Rebuild only generated subtrees; deleted source files must not survive cache reuse.
    for name in ["wave_curve", "oscillators"] {
        let generated = output.join(name);
        if generated.exists() {
            fs::remove_dir_all(generated).unwrap();
        }
    }
    for filename in [
        "dsp.rs",
        "oversampling.rs",
        "performance.rs",
        "wave_curve.rs",
    ] {
        prepare(&root.join("src").join(filename), &output.join(filename));
    }
    tree(&root.join("src/wave_curve"), &output.join("wave_curve"));
    tree(
        &root.join("src/oscillators/va"),
        &output.join("oscillators/va"),
    );
    let modules = format!(
        "#[path = {:?}] mod dsp;\n#[path = {:?}] mod oversampling;\n#[path = {:?}] mod performance;\n#[path = {:?}] mod wave_curve;\nmod oscillators {{ #[path = {:?}] pub mod va; pub(crate) use va::{{calibrate_spline_backends, Antialiasing}}; #[cfg(test)] pub(crate) use va::antialias_probe_saw; }}\n",
        output.join("dsp.rs"),
        output.join("oversampling.rs"),
        output.join("performance.rs"),
        output.join("wave_curve.rs"),
        output.join("oscillators/va/mod.rs"),
    );
    fs::write(output.join("modules.rs"), modules).unwrap();
}

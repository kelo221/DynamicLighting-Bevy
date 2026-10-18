//! Repository rules from AGENTS.md: the 400-line file limit, and no Bevy lights, banned
//! `bevy_pbr` shader modules or Unity reimplementations.

use std::fs;
use std::path::{Path, PathBuf};

const MAX_LINES: usize = 400;
const SOURCE_DIRS: [&str; 5] = ["src", "tests", "examples", "benches", "features"];
const SOURCE_EXTENSIONS: [&str; 4] = ["rs", "wesl", "wgsl", "feature"];

/// Identifiers that must not appear in code: Bevy light, sky and environment types, and Unity
/// reimplementations.
const FORBIDDEN_IDENTIFIERS: [&str; 18] = [
    "PointLight",
    "SpotLight",
    "DirectionalLight",
    "RectLight",
    "AmbientLight",
    "GlobalAmbientLight",
    "EnvironmentMapLight",
    "GeneratedEnvironmentMapLight",
    "Skybox",
    "Atmosphere",
    "IrradianceVolume",
    "LightProbe",
    "Lightmap",
    "CascadeShadowConfig",
    "UnityRandom",
    "unity_random",
    "PerlinNoise",
    "UnityObjectToClipPos",
];

/// `bevy_pbr` shader modules that carry Bevy's lighting, shadows or clustering.
const BANNED_PBR_MODULES: [&str; 8] = [
    "pbr_functions",
    "lighting",
    "shadows",
    "shadow_sampling",
    "clustered_forward",
    "fog",
    "pbr_lighting",
    "pbr_ambient",
];

/// Shaders are WESL; these are the old naga_oil preprocessor directives.
const NAGA_OIL_DIRECTIVES: [&str; 7] = [
    "#import",
    "#define_import_path",
    "#ifdef",
    "#ifndef",
    "#if ",
    "#else",
    "#endif",
];

/// Only the glTF importer may touch `StandardMaterial`, to convert it.
const STANDARD_MATERIAL_ALLOWED_IN: &str = "src/gltf_import/";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn source_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in SOURCE_DIRS {
        collect(&repo_root().join(dir), &mut files);
    }
    files.sort();
    files
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, files);
        } else if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| SOURCE_EXTENSIONS.contains(&ext))
        {
            files.push(path);
        }
    }
}

fn relative(path: &Path) -> String {
    let relative = path.strip_prefix(repo_root()).unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}

fn identifiers(line: &str) -> impl Iterator<Item = &str> {
    line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|token| !token.is_empty())
}

#[test]
fn source_files_stay_within_the_line_limit() {
    let offenders: Vec<String> = source_files()
        .iter()
        .filter_map(|path| {
            let lines = fs::read_to_string(path).ok()?.lines().count();
            (lines > MAX_LINES).then(|| format!("{} ({lines} lines)", relative(path)))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "over {MAX_LINES} lines: {offenders:#?}"
    );
}

#[test]
fn code_avoids_bevy_lights_and_unity_reimplementations() {
    let this_file = relative(Path::new(file!()));
    let mut offenders = Vec::new();
    for path in source_files() {
        let name = relative(&path);
        if name.ends_with(&this_file) {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            for token in identifiers(line) {
                let standard_material =
                    token == "StandardMaterial" && !name.starts_with(STANDARD_MATERIAL_ALLOWED_IN);
                if FORBIDDEN_IDENTIFIERS.contains(&token) || standard_material {
                    offenders.push(format!("{name}:{}: {token}", number + 1));
                }
            }
            let digits: String = line.chars().filter(char::is_ascii_digit).collect();
            if digits.contains("1812433253") {
                offenders.push(format!("{name}:{}: Unity seed constant", number + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "forbidden identifiers: {offenders:#?}"
    );
}

#[test]
fn shaders_do_not_import_bevy_lighting() {
    let mut offenders = Vec::new();
    for path in source_files() {
        let name = relative(&path);
        if !(name.ends_with(".wesl") || name.ends_with(".wgsl")) {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim_start();
            let directive = NAGA_OIL_DIRECTIVES
                .iter()
                .any(|directive| line.starts_with(directive));
            if directive || line.contains("#{") {
                offenders.push(format!("{name}:{}: naga_oil syntax", number + 1));
            }
            if line.starts_with("import") && line.contains("bevy_pbr") {
                let banned = identifiers(line).find(|token| BANNED_PBR_MODULES.contains(token));
                if let Some(module) = banned {
                    offenders.push(format!("{name}:{}: bevy_pbr::{module}", number + 1));
                }
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "banned shader imports: {offenders:#?}"
    );
}

/// Camera sphere tests go through `shared::sees_sphere`, which ignores the far plane that
/// Bevy's infinite projections never clip at.
#[test]
fn camera_sphere_tests_ignore_the_far_plane() {
    let this_file = relative(Path::new(file!()));
    let mut offenders = Vec::new();
    for path in source_files() {
        let name = relative(&path);
        if name == "src/shared/frustum.rs" || name.ends_with(&this_file) || !name.ends_with(".rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        for (number, line) in text.lines().enumerate() {
            if identifiers(line).any(|token| token == "intersects_sphere") {
                offenders.push(format!("{name}:{}", number + 1));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "use shared::sees_sphere: {offenders:#?}"
    );
}

#[test]
fn bevy_is_not_patched() {
    let manifest = fs::read_to_string(repo_root().join("Cargo.toml")).unwrap();
    assert!(
        !manifest.contains("[patch."),
        "Cargo.toml must not patch crates"
    );
}

#[test]
fn the_scanner_sees_the_crate() {
    let files: Vec<String> = source_files().iter().map(|p| relative(p)).collect();
    assert!(files.iter().any(|f| f == "src/lib.rs"));
    assert!(files.iter().any(|f| f == "src/shared/shaders/types.wesl"));
}

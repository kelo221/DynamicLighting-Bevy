//! Memory check for loading a big `.dynlight`: writes a synthetic bake, then loads and applies it
//! in a headless app. Measure the second step's peak RSS with `/usr/bin/time -l` (macOS) or
//! `/usr/bin/time -v` (Linux):
//!
//! ```text
//! cargo run --release --example bake_load_memory -- gen /tmp/big.dynlight 400
//! /usr/bin/time -l cargo run --release --example bake_load_memory -- load /tmp/big.dynlight
//! ```
//!
//! `gen` takes the unpacked size in MB. `load` has no GPU: it uploads into main-world
//! `ShaderBuffer` assets, so what it shows is the loader and apply copies.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::render::storage::ShaderBuffer;
use bevy::transform::TransformPlugin;
use dynlight::bake::{BakedMesh, BakedScene, DynLightBake, encode};
use dynlight::baked_lighting::{ActiveBake, BAKED_AUX, BAKED_TRIANGLES};
use dynlight::prelude::*;
use dynlight::shared::BakeId;

const MESH_WORDS: usize = 1 << 20;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(mode), Some(path)) = (args.next(), args.next()) else {
        eprintln!("usage: bake_load_memory gen <path> <unpacked MB> | load <path>");
        return;
    };
    let path = PathBuf::from(path);
    match mode.as_str() {
        "gen" => generate(
            &path,
            args.next().and_then(|mb| mb.parse().ok()).unwrap_or(400),
        ),
        "load" => load(&path),
        _ => eprintln!("unknown mode {mode}"),
    }
}

fn generate(path: &Path, megabytes: usize) {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut word = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        // Mostly small values, like shadow bits, so the gzip ratio is realistic.
        ((state >> 40) as u32) & 0x0F0F
    };
    let mesh_count = (megabytes * 1024 * 1024 / 4).div_ceil(MESH_WORDS);
    let meshes = (0..mesh_count as u64)
        .map(|i| BakedMesh {
            id: BakeId(i + 1),
            content_hash: i,
            resolution: 16,
            triangles: std::iter::once(4 * 1000)
                .chain((1..MESH_WORDS).map(|_| word()))
                .collect(),
            occludes: true,
        })
        .collect();
    let scene = BakedScene {
        meshes,
        ..default()
    };
    let bytes = encode(&scene);
    std::fs::write(path, &bytes).expect("write the synthetic bake");
    println!(
        "wrote {} meshes, {} MB unpacked, {} MB on disk",
        mesh_count,
        (mesh_count * MESH_WORDS * 4) >> 20,
        bytes.len() >> 20
    );
}

/// Current resident memory in MB, from `ps` (Unix).
fn rss_mb() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("run ps");
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<u64>()
        .unwrap_or(0)
        / 1024
}

fn load(path: &Path) {
    let dir = path
        .parent()
        .expect("a file path")
        .to_string_lossy()
        .into_owned();
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin {
            file_path: dir,
            ..default()
        },
        TransformPlugin,
    ))
    .init_asset::<Mesh>()
    .init_asset::<ShaderBuffer>()
    .add_plugins(DynamicLightingPlugin);
    app.finish();
    app.cleanup();
    let mut buffers = app.world_mut().resource_mut::<Assets<ShaderBuffer>>();
    for handle in [&BAKED_TRIANGLES, &BAKED_AUX] {
        buffers
            .insert(handle, ShaderBuffer::from(vec![0u32]))
            .unwrap();
    }

    let name = path
        .file_name()
        .expect("a file path")
        .to_string_lossy()
        .into_owned();
    let handle: Handle<DynLightBake> = app.world().resource::<AssetServer>().load(name);
    let start = std::time::Instant::now();
    while !app
        .world()
        .resource::<Assets<DynLightBake>>()
        .contains(&handle)
    {
        app.update();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    println!("loaded in {:?}, rss {} MB", start.elapsed(), rss_mb());
    app.insert_resource(ActiveBake(Some(handle)));
    let applying = std::time::Instant::now();
    app.update();
    println!("apply + upload frame took {:?}", applying.elapsed());
    app.update();
    println!("rss after apply {} MB", rss_mb());
}

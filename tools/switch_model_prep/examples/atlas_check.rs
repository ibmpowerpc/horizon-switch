//! Read-only check of the exact materials baked for the nine-model scene.
//! Aggregate JSON goes to stdout; no reports or resource files are written.
use craftrustnx_model_prep::scene_inputs::{hash, load_manifest, pinned, read, request, Result};
use craftrustnx_model_prep::{bake_requested, build_static_atlas, AtlasLimits};
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::PathBuf};
fn run() -> Result<()> {
    let mut mesh = PathBuf::from(".local/switch-models/26.3/terrain-probe-mesh.json");
    let mut manifest = PathBuf::from(".local/switch-assets/26.3/manifest.json");
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--help" {
            println!("atlas_check [--mesh FILE] [--manifest FILE]");
            return Ok(());
        }
        let v = PathBuf::from(args.next().ok_or("missing option value")?);
        match a.as_str() {
            "--mesh" => mesh = v,
            "--manifest" => manifest = v,
            _ => return Err("unknown option".into()),
        }
    }
    let raw = read(&mesh, 256 * 1024)?;
    let scene: Value = serde_json::from_slice(&raw)?;
    let (files, manifest_hash) = load_manifest(&manifest)?;
    if !pinned(&scene) || scene["asset_manifest_sha256"] != manifest_hash {
        return Err("scene pin/manifest mismatch".into());
    }
    let samples = scene["samples"].as_array().ok_or("missing samples")?;
    if samples.is_empty() || samples.len() > 256 {
        return Err("sample count limit".into());
    }
    let mut requests = BTreeSet::new();
    for sample in samples {
        let signatures = sample["signatures"]
            .as_array()
            .ok_or("missing signature objects")?;
        if signatures.is_empty() || signatures.len() > 64 {
            return Err("signature count limit".into());
        }
        for sig in signatures {
            requests.insert(request(sig)?);
        }
    }
    let baked = bake_requested(&files, &requests.into_iter().collect::<Vec<_>>())?;
    let sprites: BTreeSet<_> = baked
        .iter()
        .flat_map(|m| m.quads.iter().map(|q| q.material.sprite.clone()))
        .collect();
    if sprites.is_empty() {
        return Err("no baked sprite references".into());
    }
    let atlas = build_static_atlas(
        &files,
        &sprites.into_iter().collect::<Vec<_>>(),
        AtlasLimits::default(),
    )?;
    let mut low = [f32::INFINITY; 2];
    let mut high = [f32::NEG_INFINITY; 2];
    let mut outside_exact = 0usize;
    let mut outside_tolerance = 0usize;
    let mut forced = 0usize;
    for q in baked.iter().flat_map(|m| &m.quads) {
        forced += usize::from(q.material.force_translucent);
        outside_exact += usize::from(q.uv.iter().flatten().any(|u| !(0.0..=1.0).contains(u)));
        outside_tolerance += usize::from(
            q.uv.iter()
                .flatten()
                .any(|u| !(-1e-6..=1.0 + 1e-6).contains(u)),
        );
        for uv in q.uv {
            for axis in 0..2 {
                low[axis] = low[axis].min(uv[axis]);
                high[axis] = high[axis].max(uv[axis]);
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "minecraft":"26.3","protocol":777,"asset_manifest_sha256":manifest_hash,"scene_sha256":hash(&raw),
            "samples":samples.len(),"baked_models":baked.len(),"baked_quads":baked.iter().map(|m|m.quads.len()).sum::<usize>(),
            "atlas":{"width":atlas.width,"height":atlas.height,"rgba_bytes":atlas.rgba.len(),"sprites":atlas.sprites.len(),
            "alpha_sprites":atlas.sprites.values().filter(|s|s.has_alpha).count(),"partial_alpha_sprites":atlas.sprites.values().filter(|s|s.has_partial_alpha).count()},
            "forced_translucent_quads":forced,"baked_uv":{"min":low,"max":high,"quads_outside_exact_unit":outside_exact,"quads_outside_unit_with_1e_minus_6_tolerance":outside_tolerance}
        }))?
    );
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("atlas check: {e}");
        std::process::exit(1);
    }
}

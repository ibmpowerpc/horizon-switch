//! Static nine-model texture preview. Source assets stay in ignored .local/.
//! Uses the verified 0.8 sample signatures and upstream quad baking; neutral
//! white tint, no biome/world light, animation or weighted-RNG parity claim.
use craftrustnx_model_prep::scene_inputs::{hash, load_manifest, pinned, read, request, PIN};
use craftrustnx_model_prep::{bake_requested, build_static_atlas, AtlasLimits};
use horizon_assets::bake::RenderType;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[derive(Clone, Copy, Debug)]
struct Vertex {
    p: [f32; 3],
    uv: [f32; 2],
    rgba: u32,
}
fn encode(groups: &[Vec<Vertex>; 3], width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let count: usize = groups.iter().map(Vec::len).sum();
    if count == 0
        || count > 262140
        || groups.iter().any(|g| !g.len().is_multiple_of(6))
        || !width.is_power_of_two()
        || !height.is_power_of_two()
        || width > 1024
        || height > 1024
        || rgba.len() != width as usize * height as usize * 4
    {
        return Err("scene bounds/count/layout".into());
    }
    let draws = groups.iter().filter(|g| !g.is_empty()).count();
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for v in groups.iter().flatten() {
        for a in 0..3 {
            if !v.p[a].is_finite() || v.p[a].abs() > 1024.0 {
                return Err("invalid position".into());
            }
            low[a] = low[a].min(v.p[a]);
            high[a] = high[a].max(v.p[a]);
        }
        if !v
            .uv
            .iter()
            .all(|u| u.is_finite() && (0.0..=1.0).contains(u))
        {
            return Err("invalid UV".into());
        }
    }
    let mut out = b"CTTEX001".to_vec();
    for n in [777, 1, count as u32, draws as u32, width, height, 0, 0] {
        out.extend(n.to_le_bytes());
    }
    for f in low.into_iter().chain(high) {
        out.extend(f.to_le_bytes());
    }
    for v in groups.iter().flatten() {
        for f in v.p.into_iter().chain(v.uv) {
            out.extend(f.to_le_bytes());
        }
        out.extend(v.rgba.to_le_bytes());
    }
    let mut first = 0u32;
    for (mode, g) in groups.iter().enumerate() {
        if g.is_empty() {
            continue;
        }
        for n in [first, g.len() as u32, mode as u32, 0] {
            out.extend(n.to_le_bytes());
        }
        first += g.len() as u32;
    }
    out.extend(rgba);
    if out.len() > 16 * 1024 * 1024 {
        return Err("scene file limit".into());
    }
    Ok(out)
}
fn publish(path: &Path, b: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or("output parent required")?;
    fs::create_dir_all(parent)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = parent.join(format!(".texture-{}-{stamp}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        f.write_all(b)?;
        f.sync_all()?;
        fs::hard_link(&temp, path)?;
        Ok(())
    })();
    let _ = fs::remove_file(temp);
    result
}
fn run() -> Result<()> {
    let mut root = PathBuf::from(".local/switch-assets/26.3");
    let mut scene = PathBuf::from(".local/switch-models/26.3/terrain-probe-mesh.json");
    let mut output = PathBuf::from(".local/switch-models/26.3/texture-probe-scene.bin");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" {
            println!("export_texture_scene [--assets DIR] [--scene REPORT] [--output FILE]");
            return Ok(());
        }
        let v = PathBuf::from(args.next().ok_or("missing argument value")?);
        match arg.as_str() {
            "--assets" => root = v,
            "--scene" => scene = v,
            "--output" => output = v,
            _ => return Err("unknown option".into()),
        }
    }
    let report_path = output.with_extension("json");
    if fs::symlink_metadata(&output).is_ok() || fs::symlink_metadata(&report_path).is_ok() {
        return Err("output exists; no replacement".into());
    }
    let raw = read(&scene, 256 * 1024)?;
    let report: Value = serde_json::from_slice(&raw)?;
    let (files, manifest_hash) = load_manifest(&root.join("manifest.json"))?;
    if !pinned(&report) || report["asset_manifest_sha256"] != manifest_hash {
        return Err("scene pin/manifest mismatch".into());
    }
    let samples = report["samples"].as_array().ok_or("missing samples")?;
    if samples.len() != 9
        || report["grid"]["side"] != 3
        || report["grid"]["spacing"] != 2.0
        || report["grid"]["origin"] != json!([0.0, 0.0, 0.0])
    {
        return Err("expected nine-model grid".into());
    }
    let mut selections = Vec::new();
    let mut unique = BTreeSet::new();
    for s in samples {
        let sigs = s["signatures"].as_array().ok_or("missing signatures")?;
        if sigs.is_empty() || sigs.len() > 64 {
            return Err("signature count limit".into());
        }
        let reqs: Vec<_> = sigs.iter().map(request).collect::<Result<_>>()?;
        unique.extend(reqs.clone());
        selections.push(reqs);
    }
    let baked = bake_requested(&files, &unique.into_iter().collect::<Vec<_>>())?;
    let by_request: BTreeMap<_, _> = baked.iter().map(|b| (&b.request, &b.quads)).collect();
    let sprites: BTreeSet<_> = baked
        .iter()
        .flat_map(|m| m.quads.iter().map(|q| q.material.sprite.clone()))
        .collect();
    let atlas = build_static_atlas(
        &files,
        &sprites.into_iter().collect::<Vec<_>>(),
        AtlasLimits::default(),
    )?;
    let mut groups: [Vec<Vertex>; 3] = core::array::from_fn(|_| Vec::new());
    let mut tinted = 0;
    for (i, reqs) in selections.iter().enumerate() {
        for req in reqs {
            for q in *by_request.get(req).ok_or("bake request mismatch")? {
                let r = atlas
                    .sprites
                    .get(&q.material.sprite)
                    .ok_or("missing atlas sprite")?;
                if q.tint_index >= 0 {
                    tinted += 1;
                }
                let mode = match q.render_type {
                    Some(RenderType::Translucent | RenderType::Tripwire) => 2,
                    Some(RenderType::Solid) => 0,
                    Some(RenderType::Cutout | RenderType::CutoutMipped) => 1,
                    Some(RenderType::Unsupported) => return Err("unsupported render type".into()),
                    None => {
                        if q.material.force_translucent || r.has_partial_alpha {
                            2
                        } else if r.has_alpha {
                            1
                        } else {
                            0
                        }
                    }
                };
                for corner in [0, 1, 2, 0, 2, 3] {
                    let uv = q.uv[corner];
                    if !uv
                        .iter()
                        .all(|u| u.is_finite() && (-1e-6..=1.0 + 1e-6).contains(u))
                    {
                        return Err(format!(
                            "quad UV outside static sprite: {} {:?}",
                            q.material.sprite, uv
                        )
                        .into());
                    }
                    let p = q.positions[corner];
                    groups[mode].push(Vertex {
                        p: [
                            p[0] + (i % 3) as f32 * 2.0 - 2.0,
                            p[1],
                            p[2] + (i / 3) as f32 * 2.0 - 2.0,
                        ],
                        uv: [
                            (r.x as f32 + uv[0].clamp(0.0, 1.0) * r.width as f32)
                                / atlas.width as f32,
                            (r.y as f32 + uv[1].clamp(0.0, 1.0) * r.height as f32)
                                / atlas.height as f32,
                        ],
                        rgba: 0xffff_ffff,
                    });
                }
            }
        }
    }
    let bytes = encode(&groups, atlas.width, atlas.height, &atlas.rgba)?;
    let summary = json!({"schema":1,"minecraft":"26.3","protocol":777,"source_sha1":PIN,"asset_manifest_sha256":manifest_hash,"sample_report_sha256":hash(&raw),"scene_sha256":hash(&bytes),"bytes":bytes.len(),"vertices":groups.iter().map(Vec::len).sum::<usize>(),"mode_vertices":groups.iter().map(Vec::len).collect::<Vec<_>>(),"atlas_width":atlas.width,"atlas_height":atlas.height,"atlas_rgba_bytes":atlas.rgba.len(),"sprites":atlas.sprites.len(),"neutral_tint_quads":tinted,"scope":"Static preview, first weighted options; neutral tint, no biome/world light, animation, fluid or gameplay parity."});
    publish(&output, &bytes)?;
    publish(&report_path, &serde_json::to_vec_pretty(&summary)?)?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("texture export: {e}");
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_binary_layout_and_invalid_uv_refusal() {
        let v = Vertex {
            p: [0.0, 0.0, 0.0],
            uv: [0.5, 0.5],
            rgba: 0x44332211,
        };
        let mut groups = [vec![v; 6], vec![], vec![]];
        let b = encode(&groups, 1, 1, &[1, 2, 3, 4]).unwrap();
        assert_eq!(&b[..8], b"CTTEX001");
        assert_eq!(b.len(), 228);
        assert_eq!(&b[84..88], &[0x11, 0x22, 0x33, 0x44]);
        assert_eq!(
            &b[208..224],
            &[0, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        groups[0][0].uv[0] = 1.01;
        assert!(encode(&groups, 1, 1, &[1, 2, 3, 4]).is_err());
    }
}

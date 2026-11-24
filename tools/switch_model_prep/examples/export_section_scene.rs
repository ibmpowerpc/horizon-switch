//! Private captured Java section -> existing CTTEX001 static preview.
//! The network snapshot reader is the exact pure Rust codec used on Switch.
#[allow(dead_code)]
#[path = "../../../platform/switch/login-probe/codec/src/snapshot.rs"]
mod snapshot;

use craftrustnx_model_prep::bank::encode_model_bank;
use craftrustnx_model_prep::scene_inputs::{hash, load_manifest, pinned, read, PIN};
use craftrustnx_model_prep::section::{encode_scene, mesh_section};
use craftrustnx_model_prep::{bake_requested, build_static_atlas, AtlasLimits, BakeRequest};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Column {
    x: i32,
    z: i32,
    min_y: i32,
    sections: u32,
    blocks: Vec<u32>,
    biomes: Vec<u32>,
}
fn decode_column(bytes: &[u8]) -> Result<Column> {
    if bytes.len() > snapshot::MAX_FILE_SIZE {
        return Err("capture exceeds file limit".into());
    }
    let (_, _, x, z, min_y, sections, blocks, biomes) =
        snapshot::decode_header(bytes).map_err(|s| format!("capture header: {s}"))?;
    // Header validates sections/counts before allocation, then the same codec
    // as the console validates the complete file before writing either array.
    let mut column = Column {
        x,
        z,
        min_y,
        sections,
        blocks: vec![0; blocks as usize],
        biomes: vec![0; biomes as usize],
    };
    snapshot::decode_snapshot(bytes, &mut column.blocks, &mut column.biomes)
        .map_err(|s| format!("capture data: {s}"))?;
    Ok(column)
}

fn row_requests(row: &Value, models: &[Value]) -> Result<Vec<BakeRequest>> {
    let parts = row["parts"].as_array().ok_or("missing state model parts")?;
    if parts.len() > 64 {
        return Err("state part limit".into());
    }
    parts
        .iter()
        .map(|part| {
            let options = part["options"].as_array().ok_or("missing model options")?;
            if options.is_empty() || options.len() > 512 {
                return Err("model option count".into());
            }
            // Explicit static policy, preserving every multipart part. No RNG claim.
            let first = &options[0];
            if first["weight"].as_u64().is_none_or(|w| w == 0) {
                return Err("invalid selected weight".into());
            }
            let model_id = usize::try_from(first["model"].as_u64().ok_or("bad model index")?)?;
            Ok(BakeRequest {
                model: models
                    .get(model_id)
                    .and_then(Value::as_str)
                    .ok_or("model index out of range")?
                    .to_owned(),
                x: u16::try_from(first["x"].as_u64().ok_or("missing x rotation")?)?,
                y: u16::try_from(first["y"].as_u64().ok_or("missing y rotation")?)?,
                uvlock: first["uvlock"].as_bool().ok_or("missing uvlock")?,
            })
        })
        .collect()
}

// Publish both private outputs without replacing previous artifacts. Failed
// publication removes only files that this invocation successfully created.
fn publish_pair(output: &Path, scene: &[u8], report: &[u8]) -> Result<()> {
    let report_path = output.with_extension("json");
    if output == report_path
        || fs::symlink_metadata(output).is_ok()
        || fs::symlink_metadata(&report_path).is_ok()
    {
        return Err("output/report exists or paths coincide; no replacement".into());
    }
    let parent = output.parent().ok_or("output parent required")?;
    fs::create_dir_all(parent)?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let first = parent.join(format!(".section-{}-{stamp}.bin.tmp", std::process::id()));
    let second = parent.join(format!(".section-{}-{stamp}.json.tmp", std::process::id()));
    let mut created = false;
    let mut owned_temps = [false; 2];
    let result = (|| -> Result<()> {
        for (index, (path, bytes)) in [(&first, scene), (&second, report)].into_iter().enumerate() {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(path)?;
            owned_temps[index] = true;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        fs::hard_link(&first, output)?;
        created = true;
        fs::hard_link(&second, &report_path)?;
        Ok(())
    })();
    if result.is_err() && created {
        let _ = fs::remove_file(output);
    }
    for (path, owned) in [(first, owned_temps[0]), (second, owned_temps[1])] {
        if owned {
            let _ = fs::remove_file(path);
        }
    }
    result
}

fn run() -> Result<()> {
    let mut capture = None;
    let mut section = None;
    let mut bank_output = None;
    let mut assets = PathBuf::from(".local/switch-assets/26.3");
    let mut facts_path = PathBuf::from(".local/switch-state-facts/26.3/state_facts.json");
    let mut selection_path = PathBuf::from(".local/switch-models/26.3/model-selection.json");
    let mut output = PathBuf::from(".local/switch-models/26.3/captured-section-scene.bin");
    let mut seen = BTreeSet::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--help" {
            println!("export_section_scene --capture FILE --section INDEX [--assets DIR] [--facts FILE] [--selection FILE] [--output FILE] [--bank-output FILE]");
            return Ok(());
        }
        if !seen.insert(arg.clone()) {
            return Err("duplicate argument".into());
        }
        let v = args.next().ok_or("missing argument value")?;
        match arg.as_str() {
            "--capture" => capture = Some(PathBuf::from(v)),
            "--section" => section = Some(v.parse::<usize>()?),
            "--assets" => assets = v.into(),
            "--facts" => facts_path = v.into(),
            "--selection" => selection_path = v.into(),
            "--output" => output = v.into(),
            "--bank-output" => bank_output = Some(PathBuf::from(v)),
            _ => return Err("unknown argument".into()),
        }
    }
    let destination = bank_output.as_ref().unwrap_or(&output);
    if fs::symlink_metadata(destination).is_ok()
        || fs::symlink_metadata(destination.with_extension("json")).is_ok()
    {
        return Err("output already exists; no replacement".into());
    }
    let capture = read(
        &capture.ok_or("--capture is required")?,
        snapshot::MAX_FILE_SIZE,
    )?;
    let column = decode_column(&capture)?;
    let index = section.ok_or("explicit --section INDEX is required")?;
    if index >= column.sections as usize {
        return Err("section index outside capture".into());
    }
    let cells = &column.blocks[index * 4096..(index + 1) * 4096];
    let facts_bytes = read(&facts_path, 12 * 1024 * 1024)?;
    let facts: Value = serde_json::from_slice(&facts_bytes)?;
    let meta = &facts["metadata"];
    if meta["schema"] != 2
        || meta["version"] != "26.3"
        || meta["protocol"] != 777
        || meta["jar_sha1"] != PIN
    {
        return Err("state facts pin mismatch".into());
    }
    let states = facts["states"].as_array().ok_or("missing state facts")?;
    if states.is_empty()
        || states.len() > 65536
        || meta["state_count"].as_u64() != Some(states.len() as u64)
        || states
            .iter()
            .enumerate()
            .any(|(i, s)| s["id"].as_u64() != Some(i as u64))
    {
        return Err("state facts count/IDs".into());
    }
    // Validate the entire capture's IDs against this exact version's table.
    if column.blocks.iter().any(|id| *id as usize >= states.len()) {
        return Err("capture contains unknown state IDs".into());
    }
    let selection_bytes = read(&selection_path, 32 * 1024 * 1024)?;
    let selection: Value = serde_json::from_slice(&selection_bytes)?;
    let (files, manifest_hash) = load_manifest(&assets.join("manifest.json"))?;
    if !pinned(&selection)
        || selection["facts_sha256"] != hash(&facts_bytes)
        || selection["asset_manifest_sha256"] != manifest_hash
    {
        return Err("selection pin/input hashes mismatch".into());
    }
    let rows = selection["states"]
        .as_array()
        .ok_or("missing selection rows")?;
    let model_ids = selection["models"]
        .as_array()
        .ok_or("missing model table")?;
    if rows.len() != states.len()
        || model_ids.len() > 16384
        || rows
            .iter()
            .enumerate()
            .any(|(i, s)| s["id"].as_u64() != Some(i as u64))
    {
        return Err("selection row count/IDs".into());
    }
    let mut requests = BTreeMap::new();
    let mut unique = BTreeSet::new();
    let mut counts = BTreeMap::<u32, usize>::new();
    for id in cells {
        *counts.entry(*id).or_default() += 1;
    }
    let mut static_choices = 0;
    let mut invisible_cells = 0;
    for (&id, &count) in &counts {
        let fact = &states[id as usize];
        if fact["fluid_empty"].as_bool() != Some(true) {
            return Err(format!(
                "state {id}: nonempty/unknown fluid is unsupported; no geometry substituted"
            )
            .into());
        }
        let shape = fact["render_shape"]
            .as_str()
            .ok_or("missing render shape")?;
        let selected = row_requests(&rows[id as usize], model_ids)?;
        match shape {
            "INVISIBLE" if selected.is_empty() => invisible_cells += count,
            "MODEL" => {
                for part in rows[id as usize]["parts"]
                    .as_array()
                    .ok_or("missing parts")?
                {
                    if part["options"].as_array().ok_or("missing options")?.len() > 1 {
                        static_choices += count;
                    }
                }
            }
            _ => return Err(format!("state {id}: unsupported render shape {shape}").into()),
        }
        unique.extend(selected.clone());
        requests.insert(id, selected);
    }
    let baked = bake_requested(&files, &unique.into_iter().collect::<Vec<_>>())?;
    let by_request: BTreeMap<_, _> = baked.iter().map(|b| (&b.request, &b.quads)).collect();
    let mut models = BTreeMap::new();
    for (&id, selected) in &requests {
        let mut quads = Vec::new();
        for req in selected {
            quads.extend(
                (*by_request.get(req).ok_or("missing baked request")?)
                    .iter()
                    .cloned(),
            );
        }
        models.insert(id, quads);
    }
    let sprites: BTreeSet<_> = baked
        .iter()
        .flat_map(|b| b.quads.iter().map(|q| q.material.sprite.clone()))
        .collect();
    let atlas = build_static_atlas(
        &files,
        &sprites.into_iter().collect::<Vec<_>>(),
        AtlasLimits::default(),
    )?;
    let mesh = mesh_section(cells, &models, &atlas)?;
    let bytes = encode_scene(&mesh, &atlas)?;
    let mut summary = json!({
        "schema":1,"minecraft":"26.3","protocol":777,"source_sha1":PIN,
        "asset_manifest_sha256":manifest_hash,"facts_sha256":hash(&facts_bytes),
        "selection_sha256":hash(&selection_bytes),"scene_sha256":hash(&bytes),"bytes":bytes.len(),
        "capture":{"sha256":hash(&capture),"bytes":capture.len(),"chunk_x":column.x,"chunk_z":column.z,
            "min_y":column.min_y,"sections":column.sections},
        "section_index":index,"world_min_y":column.min_y+index as i32*16,"cells":4096,
        "biome_cells":column.biomes[index*64..(index+1)*64].len(),
        "unique_states":counts.len(),"cells_with_geometry":mesh.cells_with_geometry,
        "empty_cells":mesh.empty_cells,"invisible_cells":invisible_cells,
        "first_weighted_part_instances":static_choices,"baked_requests":baked.len(),
        "quads":mesh.source_quads,"neutral_tint_quads":mesh.tinted_quads,
        "vertices":mesh.groups.iter().map(Vec::len).sum::<usize>(),
        "mode_vertices":mesh.groups.iter().map(Vec::len).collect::<Vec<_>>(),
        "atlas_width":atlas.width,"atlas_height":atlas.height,"sprites":atlas.sprites.len(),
        "scope":"Static captured section in local coordinates. First weighted options, original multipart models. Neutral tint, no world light, fluid, animation, block entity rendering, jitter/RNG, neighbor occlusion, live updates or gameplay."
    });
    if bank_output.is_some() {
        let bank = encode_model_bank(&models, &atlas)?;
        summary["artifact_kind"] = json!("CTBANK01 model bank for on-device meshing");
        summary["bank_sha256"] = json!(hash(&bank));
        summary["bank_bytes"] = json!(bank.len());
        summary["bank_states"] = json!(models.len());
        summary["bank_template_vertices"] = json!(u32::from_le_bytes(bank[20..24].try_into()?));
        publish_pair(destination, &bank, &serde_json::to_vec_pretty(&summary)?)?;
    } else {
        publish_pair(destination, &bytes, &serde_json::to_vec_pretty(&summary)?)?;
    }
    println!("{}", serde_json::to_string_pretty(&summary)?);
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("section export: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_uses_the_console_codec_and_exact_extent() {
        let mut file = b"CTCOL001".to_vec();
        for word in [
            777u32,
            1,
            (-2i32) as u32,
            3,
            (-64i32) as u32,
            1,
            4096,
            64,
            0,
            0,
        ] {
            file.extend(word.to_le_bytes());
        }
        for i in 0u32..4160 {
            file.extend((0x0102_0304 + i).to_le_bytes());
        }
        let column = decode_column(&file).unwrap();
        assert_eq!(
            (column.x, column.z, column.min_y, column.sections),
            (-2, 3, -64, 1)
        );
        assert_eq!(column.blocks[1], 0x0102_0305);
        assert_eq!(column.biomes[0], 0x0102_1304);
        assert!(decode_column(&file[..file.len() - 1]).is_err());
        file.push(0);
        assert!(decode_column(&file).is_err());
    }
    #[test]
    fn every_part_is_kept_and_first_alternative_is_explicit() {
        let option = |n| json!({"model":n,"x":0,"y":90,"uvlock":false,"weight":1});
        let row = json!({"parts":[{"options":[option(0),option(1)]},{"options":[option(1)]}]});
        let models = vec![json!("minecraft:block/a"), json!("minecraft:block/b")];
        let got = row_requests(&row, &models).unwrap();
        assert_eq!(
            got.iter().map(|r| r.model.as_str()).collect::<Vec<_>>(),
            vec!["minecraft:block/a", "minecraft:block/b"]
        );
        assert!(row_requests(&row, &models[..1]).is_err());
        assert!(row_requests(&json!({"parts":[{"options":[]}]}), &models).is_err());
    }
    #[test]
    fn publication_never_replaces_a_previous_scene() {
        let path = std::env::temp_dir().join(format!(
            "switch-section-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = path.join("scene.bin");
        publish_pair(&output, b"original", b"report").unwrap();
        assert!(publish_pair(&output, b"replacement", b"other").is_err());
        assert_eq!(fs::read(&output).unwrap(), b"original");
        assert_eq!(fs::read(output.with_extension("json")).unwrap(), b"report");
        fs::remove_dir_all(path).unwrap();
    }
}

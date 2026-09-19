#![forbid(unsafe_code)]

use corum_assets::cdb::{self, Record};
use corum_assets::chr::ChrManifest;
use corum_assets::lightmap::LightmapFile;
use corum_assets::map_script::MapScript;
use corum_assets::model::ModelFile;
use corum_assets::motion::MotionFile;
use corum_assets::pose::{Skeleton, transform_point};
use corum_assets::stm::StaticModelFile;
use corum_assets::tables;
use corum_assets::ttb::TileMap;
use corum_assets::vcl::VertexColors;
use corum_assets::{PakArchive, PakError};
use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    let Some(command) = arguments.first().map(String::as_str) else {
        return Err(usage());
    };

    match command {
        "info" if arguments.len() == 2 => info(&arguments[1]),
        "list" if arguments.len() == 2 => list(&arguments[1]),
        "extract" if arguments.len() == 4 => extract(&arguments[1], &arguments[2], &arguments[3]),
        "extract-all" if arguments.len() == 3 => extract_all(&arguments[1], &arguments[2]),
        "chr-info" if arguments.len() == 2 => chr_info(&arguments[1]),
        "mod-info" if arguments.len() == 2 => mod_info(&arguments[1]),
        "mod-to-obj" if arguments.len() == 3 => mod_to_obj(&arguments[1], &arguments[2]),
        "anm-info" if arguments.len() == 2 => anm_info(&arguments[1]),
        "ttb-info" if arguments.len() == 2 => ttb_info(&arguments[1]),
        "map-info" if arguments.len() == 2 => map_info(&arguments[1]),
        "stm-info" if arguments.len() == 2 => stm_info(&arguments[1]),
        "vcl-info" if arguments.len() == 3 => vcl_info(&arguments[1], &arguments[2]),
        "pose-check" if arguments.len() == 3 => pose_check(&arguments[1], &arguments[2]),
        "lm-info" if arguments.len() == 3 => lm_info(&arguments[1], &arguments[2]),
        "cdb-info" if arguments.len() == 2 => cdb_info(&arguments[1]),
        "cdb-export-tsv" if arguments.len() == 3 => cdb_export_tsv(&arguments[1], &arguments[2]),
        "tsv-to-cdb" if arguments.len() == 4 => {
            tsv_to_cdb(&arguments[1], &arguments[2], &arguments[3])
        }
        "erd-dump" if arguments.len() == 2 => erd_dump(&arguments[1]),
        "cdb-decode-all" if arguments.len() == 3 => cdb_decode_all(&arguments[1], &arguments[2]),
        "help" | "--help" | "-h" => {
            println!("{}", usage());
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn cdb_info(path: &str) -> Result<(), String> {
    let decoded = cdb::decode(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    println!("decoded_bytes: {}", decoded.len());
    let name = Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match name.as_str() {
        "level.cdb" => cdb_rows::<cdb::LevelExp>(&decoded, |row| {
            format!("level={} exp={}", row.level, row.exp)
        }),
        "guardianlevel.cdb" | "guardianexp.cdb" => {
            cdb_rows::<cdb::GuardianLevelExp>(&decoded, |row| format!("{row:?}"))
        }
        "npctable.cdb" => cdb_rows::<cdb::NpcTable>(&decoded, |row| {
            format!(
                "id={} name={} type={} says={}",
                row.id,
                row.name.lossy(),
                row.kind,
                row.messages[0].lossy()
            )
        }),
        "cptable.cdb" => cdb_rows::<cdb::CpTable>(&decoded, |row| {
            format!(
                "id={} name={} class={} values={:?}",
                row.id,
                row.english_name.lossy(),
                row.class,
                row.values
            )
        }),
        "itemstore.cdb" => cdb_rows::<cdb::ItemStore>(&decoded, |row| format!("{row:?}")),
        "itemresource.cdb" => cdb_rows::<cdb::ItemResource>(&decoded, |row| {
            format!(
                "id={} icon={} model={} type={}",
                row.id,
                row.icon_file.lossy(),
                row.model_file.lossy(),
                row.resource_type
            )
        }),
        "skillresource.cdb" => cdb_rows::<cdb::SkillResource>(&decoded, |row| {
            format!(
                "id={} icon={} kind={}",
                row.id,
                row.icon_file.lossy(),
                row.kind
            )
        }),
        "itemoption.cdb" => cdb_rows::<cdb::ItemOption>(&decoded, |row| {
            format!(
                "id={} count={} first={}",
                row.id,
                row.count,
                row.options[0].lossy()
            )
        }),
        "help.cdb" | "helpinfo.cdb" => cdb_rows::<cdb::HelpInfo>(&decoded, |row| {
            format!(
                "id={} text={} at=({},{})",
                row.id,
                row.text.lossy(),
                row.left,
                row.top
            )
        }),
        "dungeonproductionitemminmax.cdb" => {
            cdb_rows::<cdb::DungeonProductionItemRange>(&decoded, |row| format!("{row:?}"))
        }
        "baseclassinfo.cdb" => cdb_rows::<cdb::BaseClassInfo>(&decoded, |row| format!("{row:?}")),
        _ => match cdb::TextPool::parse(&decoded) {
            Ok(pool) => {
                println!("kind: text pool, {} entries", pool.entries.len());
                for entry in pool.entries.iter().take(10) {
                    println!("  id={} text={}", entry.id, entry.text.lossy());
                }
                Ok(())
            }
            Err(_) => {
                println!("kind: table without a typed parser yet");
                Ok(())
            }
        },
    }
}

fn cdb_rows<T: Record>(decoded: &[u8], show: impl Fn(&T) -> String) -> Result<(), String> {
    let rows = cdb::parse_table::<T>(decoded).map_err(|error| error.to_string())?;
    println!("kind: {} records of {} bytes", rows.len(), T::SIZE);
    for row in rows.iter().take(10) {
        println!("  {}", show(row));
    }
    Ok(())
}

fn cdb_export_tsv(directory: &str, output: &str) -> Result<(), String> {
    fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let mut done = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let is_cdb = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("cdb"));
        let Some(schema) = is_cdb.then(|| tables::schema_for(&stem)).flatten() else {
            continue;
        };
        let decoded = cdb::decode(&fs::read(&path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let tsv = schema
            .to_tsv(&decoded)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let rows = tsv.lines().count() - 1;
        fs::write(Path::new(output).join(format!("{stem}.tsv")), tsv)
            .map_err(|error| error.to_string())?;
        done.push(format!(
            "{stem} ({rows} rows, {} columns)",
            schema.columns().len()
        ));
    }
    done.sort();
    for line in &done {
        println!("  {line}");
    }
    println!("exported {} tables to {output}", done.len());
    Ok(())
}

fn tsv_to_cdb(table: &str, tsv_path: &str, cdb_path: &str) -> Result<(), String> {
    let schema = tables::schema_for(table).ok_or_else(|| format!("no schema for `{table}`"))?;
    let tsv = fs::read_to_string(tsv_path).map_err(|error| error.to_string())?;
    let body = schema.from_tsv(&tsv).map_err(|error| error.to_string())?;
    fs::write(cdb_path, cdb::encode(&body)).map_err(|error| error.to_string())?;
    println!("wrote {cdb_path} ({} bytes of table data)", body.len());
    Ok(())
}

fn erd_dump(path: &str) -> Result<(), String> {
    let entries = corum_assets::erd::parse(&fs::read(path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    for entry in &entries {
        println!("0x{:08X}	{}", entry.id, entry.path);
    }
    Ok(())
}

fn cdb_decode_all(directory: &str, output: &str) -> Result<(), String> {
    fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let mut done = 0;
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path
            .extension()
            .is_none_or(|extension| !extension.eq_ignore_ascii_case("cdb"))
        {
            continue;
        }
        let decoded = cdb::decode(&fs::read(&path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let name = path.file_stem().unwrap_or_default().to_string_lossy();
        fs::write(Path::new(output).join(format!("{name}.bin")), decoded)
            .map_err(|error| error.to_string())?;
        done += 1;
    }
    println!("decoded {done} tables into {output}");
    Ok(())
}

fn ttb_info(path: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let map = TileMap::parse(&bytes).map_err(|error| error.to_string())?;
    let walkable = map.tiles.iter().filter(|tile| tile.is_walkable()).count();
    println!("dimensions: {}x{}", map.width, map.height);
    println!("tile_size: {}", map.tile_size);
    println!("declared_objects: {}", map.declared_object_count);
    println!("walkable_tiles: {walkable}/{}", map.tiles.len());
    println!("declared_sections: {:?}", map.declared_section_count);
    println!("trailing_bytes: {}", map.trailing_bytes);
    Ok(())
}

fn map_info(path: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let map = MapScript::parse(&bytes).map_err(|error| error.to_string())?;
    println!("bounds_min: {:?}", map.bounds_min);
    println!("bounds_max: {:?}", map.bounds_max);
    println!("static_model: {:?}", map.static_model);
    println!("height_field: {:?}", map.height_field);
    println!("objects: {}", map.objects.len());
    println!("lights: {}", map.lights.len());
    for object in map.objects.iter().take(20) {
        println!(
            "  {} id={} position={:?} scale={:?}",
            object.resource, object.id, object.position, object.scale
        );
    }
    Ok(())
}

fn lm_info(lm_path: &str, stm_path: &str) -> Result<(), String> {
    let lightmaps = LightmapFile::parse(&fs::read(lm_path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let stm = StaticModelFile::parse(&fs::read(stm_path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let objects: Vec<_> = stm
        .objects
        .iter()
        .filter(|object| object.object_type == 3)
        .collect();
    println!("lightmaps: {}", lightmaps.maps.len());
    println!("lightmapped_objects: {}", objects.len());
    for (index, object) in objects.iter().enumerate() {
        let map = lightmaps.maps.get(index);
        let agrees = match (map, object.lightmap) {
            (Some(map), Some(descriptor)) => {
                map.first_field == descriptor.first_field
                    && map.width == descriptor.width
                    && map.height == descriptor.height
            }
            _ => false,
        };
        println!(
            "  #{index} {} -> {} {}",
            object.name,
            map.map_or_else(
                || "missing".to_owned(),
                |map| format!(
                    "{}x{} (first field {})",
                    map.width, map.height, map.first_field
                )
            ),
            if agrees { "header matches" } else { "MISMATCH" }
        );
    }
    Ok(())
}

/// Poses a model with a motion and reports how well the pieces agree: tracks matched, how far
/// the frame-0 pose is from the bind pose, and how well the skin reproduces the mesh.
fn pose_check(model_path: &str, motion_path: &str) -> Result<(), String> {
    let model = ModelFile::parse(&fs::read(model_path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let motion = MotionFile::parse(&fs::read(motion_path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let skeleton = Skeleton::new(&model);
    let tracks = skeleton.tracks_for(&motion);
    let animated = tracks.iter().filter(|track| track.is_some()).count();
    println!(
        "nodes: {} ({} bones), animated by the motion: {animated}",
        skeleton.node_count(),
        model.bones.len()
    );
    println!(
        "motion: {} frames at {} fps ({:.2} s)",
        motion.frame_count(),
        motion.frame_speed,
        motion.duration_seconds()
    );

    // 1. With no tracks the pose must be the bind pose, node by node and through the skin.
    let bind = skeleton.pose(&vec![None; skeleton.node_count()], 0.0);
    let (nodes_error, node_name) = largest_node_difference(&model, &skeleton, &bind);
    let (skinned, vertices, skin_error) = skin_difference(&model, &skeleton, &bind);
    println!("bind pose (no tracks): largest node difference {nodes_error:.4} ({node_name})");
    println!(
        "skin at the bind pose: {skinned} skinned meshes, {vertices} vertices, largest difference vs the stored positions {skin_error:.3}"
    );

    // 2. The first frame of a motion is a pose of its own (an idle is not the bind pose), so it
    //    is only reported: how far the animated skeleton moves away from the bind pose.
    let first = skeleton.pose(&tracks, motion.first_frame as f32);
    let (moved, moved_name) = largest_node_difference(&model, &skeleton, &first);
    println!(
        "first frame of the motion: largest node movement from the bind pose {moved:.1} ({moved_name})"
    );

    // 3. Bones are rigid: the distance from a node to its parent must not change at any frame.
    let mut worst_length = (0.0_f32, String::new(), 0_u32);
    for frame in (0..motion.frame_count()).step_by(5) {
        let posed = skeleton.pose(&tracks, motion.first_frame as f32 + frame as f32);
        for (index, node) in model.nodes.iter().enumerate() {
            let Some(parent) = skeleton.index_of_id(node.parent_id) else {
                continue;
            };
            if parent == index {
                continue;
            }
            let distance = |world: &[corum_assets::model::Matrix]| {
                (0..3)
                    .map(|axis| (world[index][3][axis] - world[parent][3][axis]).powi(2))
                    .sum::<f32>()
                    .sqrt()
            };
            let change = (distance(&posed) - distance(skeleton.bind_world())).abs();
            if change > worst_length.0 {
                worst_length = (change, node.name.clone(), frame);
            }
        }
    }
    println!(
        "bone lengths over the motion: largest change {:.3} ({} at frame {})",
        worst_length.0, worst_length.1, worst_length.2
    );
    Ok(())
}

fn largest_node_difference(
    model: &ModelFile,
    skeleton: &Skeleton,
    pose: &[corum_assets::model::Matrix],
) -> (f32, String) {
    let mut worst = (0.0_f32, String::new());
    for (index, world) in pose.iter().enumerate() {
        let bind = skeleton.bind_world()[index];
        let error = (0..3)
            .map(|axis| (world[3][axis] - bind[3][axis]).abs())
            .fold(0.0_f32, f32::max);
        if error > worst.0 {
            worst = (error, model.nodes[index].name.clone());
        }
    }
    worst
}

/// Skins every mesh with `pose` and compares with the stored positions:
/// `(skinned meshes, vertices, largest difference)`.
fn skin_difference(
    model: &ModelFile,
    skeleton: &Skeleton,
    pose: &[corum_assets::model::Matrix],
) -> (usize, usize, f32) {
    let (mut skinned, mut vertices, mut worst) = (0_usize, 0_usize, 0.0_f32);
    for mesh in &model.meshes {
        let Some(geometry) = &mesh.geometry else {
            continue;
        };
        let Some(skin) = &geometry.skin else {
            continue;
        };
        skinned += 1;
        for (index, influences) in skin.influences.iter().enumerate() {
            let mut sum = [0.0_f32; 3];
            let mut total = 0.0_f32;
            for influence in influences {
                let Some(bone) = skeleton.index_of_id(influence.bone_id) else {
                    continue;
                };
                let point = transform_point(&pose[bone], influence.offset);
                for axis in 0..3 {
                    sum[axis] += influence.weight * point[axis];
                }
                total += influence.weight;
            }
            let expected = geometry.positions[index];
            // Uninitialised memory (0xCDCDCDCD) in the file's own positions is not a skin error.
            if total > 1e-6 && expected.iter().all(|value| value.abs() < 1.0e6) {
                let error = (0..3)
                    .map(|axis| (sum[axis] / total - expected[axis]).abs())
                    .fold(0.0_f32, f32::max);
                worst = worst.max(error);
            }
            vertices += 1;
        }
    }
    (skinned, vertices, worst)
}

fn vcl_info(vcl_path: &str, stm_path: &str) -> Result<(), String> {
    let vcl = VertexColors::parse(&fs::read(vcl_path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let stm = StaticModelFile::parse(&fs::read(stm_path).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    let expected = stm.vertex_lit_vertex_count();
    println!("colors: {}", vcl.colors.len());
    println!("vertex_lit_vertices: {expected}");
    println!(
        "match: {}",
        if vcl.colors.len() == expected {
            "yes"
        } else {
            "NO"
        }
    );
    Ok(())
}

fn stm_info(path: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let model = StaticModelFile::parse(&bytes).map_err(|error| error.to_string())?;
    let face_count: usize = model
        .objects
        .iter()
        .flat_map(|object| &object.groups)
        .map(|group| group.faces.len())
        .sum();
    let mut minimum = [f32::INFINITY; 3];
    let mut maximum = [f32::NEG_INFINITY; 3];
    for position in model
        .objects
        .iter()
        .flat_map(|object| object.positions.iter())
    {
        for axis in 0..3 {
            minimum[axis] = minimum[axis].min(position[axis]);
            maximum[axis] = maximum[axis].max(position[axis]);
        }
    }
    println!("materials: {}", model.materials.len());
    println!("visual_objects: {}", model.objects.len());
    println!("skipped_objects: {}", model.skipped_objects.len());
    println!("unread_objects: {}", model.unread_object_offsets.len());
    for offset in model.unread_object_offsets.iter().take(10) {
        println!("  UNREAD object header at 0x{offset:X}");
    }
    println!("faces: {face_count}");
    println!("bounds_min: {minimum:?}");
    println!("bounds_max: {maximum:?}");
    for object in &model.objects {
        println!(
            "  {}: vertices={}, groups={}",
            object.name,
            object.positions.len(),
            object.groups.len()
        );
    }
    Ok(())
}

fn chr_info(path: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let manifest = ChrManifest::parse(&bytes).map_err(|error| error.to_string())?;
    println!("model: {}", manifest.model_file);
    println!("motions: {}", manifest.motions.len());
    for motion in manifest.motions {
        println!("  {motion}");
    }
    Ok(())
}

fn mod_info(path: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let model = ModelFile::parse(&bytes).map_err(|error| error.to_string())?;
    println!("version: {}", model.version);
    println!("nodes: {}", model.node_count);
    println!("materials: {}", model.material_count);
    for (index, material) in model.materials.iter().enumerate() {
        println!(
            "  {index}: name={}, texture={}, selector={:?}, flags=0x{:08X}",
            material.name, material.texture_name, material.selector, material.flags
        );
    }
    println!("meshes: {}", model.meshes.len());
    for mesh in &model.meshes {
        let face_count: usize = mesh
            .geometry
            .as_ref()
            .map(|geometry| {
                geometry
                    .face_groups
                    .iter()
                    .map(|group| group.faces.len())
                    .sum()
            })
            .unwrap_or(0);
        println!(
            "  {}: vertices={}, uv={}, seams={}, faces={}, exportable={}, skinned={}{}",
            mesh.name,
            mesh.vertex_count,
            mesh.texture_vertex_count,
            mesh.seam_vertex_count,
            face_count,
            mesh.geometry.is_some(),
            mesh.geometry
                .as_ref()
                .is_some_and(|geometry| geometry.skin.is_some()),
            mesh.geometry_issue
                .as_ref()
                .map(|issue| format!(", issue={issue}"))
                .unwrap_or_default()
        );
    }
    println!("bones: {}", model.bones.len());
    for bone in &model.bones {
        println!("  {bone}");
    }
    println!("unsupported_records: {}", model.unsupported_records.len());
    for record in &model.unsupported_records {
        println!(
            "  tag=0x{:08X}, bytes={}, name={}",
            record.tag, record.payload_size, record.candidate_name
        );
    }
    Ok(())
}

fn mod_to_obj(path: &str, output: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let model = ModelFile::parse(&bytes).map_err(|error| error.to_string())?;
    let obj = model.to_obj().map_err(|error| error.to_string())?;
    fs::write(output, obj).map_err(|error| error.to_string())?;
    println!("exported {output}");
    Ok(())
}

fn anm_info(path: &str) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    let motion = MotionFile::parse(&bytes).map_err(|error| error.to_string())?;
    println!("version: {}", motion.version);
    println!("name: {}", motion.name);
    println!("ticks_per_frame: {}", motion.ticks_per_frame);
    println!("frames: {}..={}", motion.first_frame, motion.last_frame);
    println!("frame_speed: {}", motion.frame_speed);
    println!("duration_ticks: {}", motion.duration_ticks);
    println!("records: {}", motion.records.len());
    for record in &motion.records {
        println!(
            "  offset=0x{:X}, bytes={}, tracks=({}/{}/{}), morph={}/{} bytes, name={}",
            record.offset,
            record.payload_size,
            record.track_24.len(),
            record.track_20.len(),
            record.track_36.len(),
            record.morph_key_count,
            record.morph_bytes,
            record.candidate_name
        );
    }
    Ok(())
}

fn info(path: &str) -> Result<(), String> {
    let pak = open(path)?;
    println!("path: {}", pak.path().display());
    println!("version: {}", pak.header().version);
    println!("flags: 0x{:08X}", pak.header().flags);
    println!("name: {}", pak.header().name);
    println!("entries: {}", pak.entries().len());
    println!("bytes: {}", pak.file_size());
    Ok(())
}

fn list(path: &str) -> Result<(), String> {
    let pak = open(path)?;
    println!("index\tsize\tdata_offset\tname");
    for entry in pak.entries() {
        println!(
            "{}\t{}\t{}\t{}",
            entry.index, entry.size, entry.data_offset, entry.name
        );
    }
    Ok(())
}

fn extract(path: &str, entry_name: &str, output_directory: &str) -> Result<(), String> {
    let pak = open(path)?;
    let output = pak
        .extract_entry(entry_name, Path::new(output_directory))
        .map_err(format_error)?;
    println!("extracted {}", output.display());
    Ok(())
}

fn extract_all(path: &str, output_directory: &str) -> Result<(), String> {
    let pak = open(path)?;
    let count = pak
        .extract_all(Path::new(output_directory))
        .map_err(format_error)?;
    println!("extracted {count} entries to {output_directory}");
    Ok(())
}

fn open(path: &str) -> Result<PakArchive, String> {
    PakArchive::open(path).map_err(format_error)
}

fn format_error(error: PakError) -> String {
    error.to_string()
}

fn usage() -> String {
    [
        "corum-assets — inspect and extract Corum Online PAK archives",
        "",
        "Usage:",
        "  corum-assets info <archive.pak>",
        "  corum-assets list <archive.pak>",
        "  corum-assets extract <archive.pak> <entry-name> <output-directory>",
        "  corum-assets extract-all <archive.pak> <output-directory>",
        "  corum-assets chr-info <manifest.chr>",
        "  corum-assets mod-info <model.mod>",
        "  corum-assets mod-to-obj <model.mod> <output.obj>",
        "  corum-assets anm-info <motion.anm>",
        "  corum-assets ttb-info <map.ttb>",
        "  corum-assets cdb-info <table.cdb>",
        "  corum-assets cdb-export-tsv <Data/Manager> <output-dir>",
        "  corum-assets tsv-to-cdb <table-name> <table.tsv> <output.cdb>",
        "  corum-assets erd-dump <resource.erd>",
        "  corum-assets cdb-decode-all <Data/Manager> <output-dir>",
        "  corum-assets map-info <scene.map>",
        "  corum-assets stm-info <scene.stm>",
        "  corum-assets vcl-info <scene.vcl> <scene.stm>",
        "  corum-assets pose-check <model.mod> <motion.anm>",
        "  corum-assets lm-info <scene.lm> <scene.stm>",
    ]
    .join("\n")
}

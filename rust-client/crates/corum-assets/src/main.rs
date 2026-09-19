#![forbid(unsafe_code)]

use corum_assets::chr::ChrManifest;
use corum_assets::model::ModelFile;
use corum_assets::motion::MotionFile;
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
        "help" | "--help" | "-h" => {
            println!("{}", usage());
            Ok(())
        }
        _ => Err(usage()),
    }
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
            "  {}: vertices={}, uv={}, seams={}, faces={}, exportable={}{}",
            mesh.name,
            mesh.vertex_count,
            mesh.texture_vertex_count,
            mesh.seam_vertex_count,
            face_count,
            mesh.geometry.is_some(),
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
    ]
    .join("\n")
}

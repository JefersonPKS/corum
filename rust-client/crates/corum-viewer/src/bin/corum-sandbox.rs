#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use corum_assets::PakArchive;
use corum_assets::dds::DecodedImage;
use corum_assets::lightmap::LightmapFile;
use corum_assets::map_script::{MapLight, MapScript};
use corum_assets::stm::StaticModelFile;
use corum_assets::ttb::TileMap;
use corum_assets::vcl::VertexColors;
use glam::{Vec2, Vec3};
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const DEFAULT_MAP: &str = r"D:\Games\CorumOnline\Data\Map\1100.ttb";
const DEFAULT_DATA_DIRECTORY: &str = r"D:\Games\CorumOnline\Data";
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn main() {
    if let Err(error) = run() {
        eprintln!("corum-sandbox: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut arguments = env::args().skip(1);
    let path = arguments
        .next()
        .map_or_else(|| PathBuf::from(DEFAULT_MAP), PathBuf::from);
    let stm_path = arguments
        .next()
        .map_or_else(|| path.with_extension("stm"), PathBuf::from);
    let bytes =
        fs::read(&path).map_err(|error| format!("could not read '{}': {error}", path.display()))?;
    let map = TileMap::parse(&bytes).map_err(|error| error.to_string())?;
    let static_model = if stm_path.is_file() {
        let bytes = fs::read(&stm_path)
            .map_err(|error| format!("could not read '{}': {error}", stm_path.display()))?;
        Some(StaticModelFile::parse(&bytes).map_err(|error| error.to_string())?)
    } else {
        eprintln!(
            "STM not found at '{}'; showing collision only",
            stm_path.display()
        );
        None
    };
    let mut textures = static_model
        .as_ref()
        .map(|model| MapTextures::load(model, &path))
        .unwrap_or_default();
    if let Some(model) = static_model.as_ref()
        && let Some(lightmaps) = load_lightmaps(&path)
    {
        textures.attach_lightmaps(model, &lightmaps);
    }
    let baked = load_baked_colors(&path, static_model.as_ref());
    let lights = load_lights(&path);
    let scene = SandboxScene::new(
        map,
        static_model.as_ref(),
        &textures,
        baked.as_ref(),
        &lights,
    )?;
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let mut application = SandboxApplication::new(path, scene, textures);
    event_loop
        .run_app(&mut application)
        .map_err(|error| error.to_string())
}

/// Reads `<map>.<extension>` next to the `.ttb`, or else from `Data\Map_light\Map_light.pak`,
/// where the lighting files of the packed maps live.
fn read_map_file(ttb_path: &Path, extension: &str) -> Option<Vec<u8>> {
    let sibling = ttb_path.with_extension(extension);
    if let Ok(bytes) = fs::read(&sibling) {
        return Some(bytes);
    }
    let name = sibling.file_name()?.to_str()?;
    // A few maps keep their lighting files in `Map_tif.pak` instead.
    ["Map_light", "Map_tif"].into_iter().find_map(|package| {
        let archive = open_data_package(ttb_path, package).ok()?;
        archive.read_entry(name).ok()
    })
}

/// Data directories to search for `Map_*` packages: the game folder that holds the `.ttb`
/// (`<data>\Map\N.ttb`), then `CORUM_DATA`, then the default installation. This lets maps
/// extracted to another folder still find their textures and lighting.
fn data_directories(ttb_path: &Path) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Ok(canonical) = ttb_path.canonicalize()
        && let Some(data) = canonical.parent().and_then(Path::parent)
    {
        directories.push(data.to_path_buf());
    }
    if let Some(configured) = env::var_os("CORUM_DATA") {
        directories.push(PathBuf::from(configured));
    }
    directories.push(PathBuf::from(DEFAULT_DATA_DIRECTORY));
    directories
}

/// Opens `<data>\<package>\<package>.pak` from the first data directory that has it.
fn open_data_package(ttb_path: &Path, package: &str) -> Result<PakArchive, String> {
    let mut last_error = format!("no data directory contains {package}");
    for directory in data_directories(ttb_path) {
        let path = directory.join(package).join(format!("{package}.pak"));
        if !path.is_file() {
            continue;
        }
        match PakArchive::open(&path) {
            Ok(archive) => return Ok(archive),
            Err(error) => last_error = format!("{}: {error}", path.display()),
        }
    }
    Err(last_error)
}

/// Baked lightmaps (`.lm`). An empty file is valid: some maps ship none.
fn load_lightmaps(ttb_path: &Path) -> Option<LightmapFile> {
    let bytes = read_map_file(ttb_path, "lm")?;
    match LightmapFile::parse(&bytes) {
        Ok(file) if file.maps.is_empty() => None,
        Ok(file) => Some(file),
        Err(error) => {
            eprintln!("ignoring lightmaps: {error}");
            None
        }
    }
}

/// Baked per-vertex lighting next to the `.ttb`. Absence is normal (not every map has one).
fn load_baked_colors(ttb_path: &Path, model: Option<&StaticModelFile>) -> Option<VertexColors> {
    let model = model?;
    let path = ttb_path.with_extension("vcl");
    let bytes = read_map_file(ttb_path, "vcl")?;
    match VertexColors::parse(&bytes) {
        Ok(colors) if colors.colors.len() == model.vertex_lit_vertex_count() => {
            eprintln!("loaded VCL: {} baked vertex colours", colors.colors.len());
            Some(colors)
        }
        Ok(colors) => {
            eprintln!(
                "ignoring '{}': {} colours for {} vertex-lit vertices",
                path.display(),
                colors.colors.len(),
                model.vertex_lit_vertex_count()
            );
            None
        }
        Err(error) => {
            eprintln!("ignoring '{}': {error}", path.display());
            None
        }
    }
}

/// `GX_LIGHT` entries from the `.map` script next to the `.ttb`.
fn load_lights(ttb_path: &Path) -> Vec<MapLight> {
    let path = ttb_path.with_extension("map");
    let Ok(bytes) = fs::read(&path) else {
        return Vec::new();
    };
    match MapScript::parse(&bytes) {
        Ok(script) => {
            eprintln!("loaded MAP: {} point lights", script.lights.len());
            script.lights
        }
        Err(error) => {
            eprintln!("ignoring '{}': {error}", path.display());
            Vec::new()
        }
    }
}

struct SandboxApplication {
    path: PathBuf,
    scene: Option<SandboxScene>,
    textures: Option<MapTextures>,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
}

impl SandboxApplication {
    fn new(path: PathBuf, scene: SandboxScene, textures: MapTextures) -> Self {
        Self {
            path,
            scene: Some(scene),
            textures: Some(textures),
            window: None,
            renderer: None,
        }
    }

    fn title(path: &Path, scene: &SandboxScene) -> String {
        format!(
            "Corum Map Viewer — {} — {} — Tab: peça | Shift+Tab: anterior | 0: tudo | F: foco | G: colisão | H: entidades | L: luzes | B: brilho",
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("map.ttb"),
            scene.selection_label()
        )
    }
}

impl ApplicationHandler for SandboxApplication {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let title = Self::title(
            &self.path,
            self.scene.as_ref().expect("scene exists before resume"),
        );
        let attributes = Window::default_attributes()
            .with_title(title)
            .with_inner_size(LogicalSize::new(1200.0, 820.0));
        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::new(window),
            Err(error) => {
                eprintln!("failed to create window: {error}");
                event_loop.exit();
                return;
            }
        };
        let scene = self.scene.take().expect("scene is initialized once");
        let textures = self.textures.take().expect("textures are uploaded once");
        let renderer = match pollster::block_on(Renderer::new(window.clone(), scene, textures)) {
            Ok(renderer) => renderer,
            Err(error) => {
                eprintln!("failed to initialize renderer: {error}");
                event_loop.exit();
                return;
            }
        };
        window.request_redraw();
        self.renderer = Some(renderer);
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let (Some(window), Some(renderer)) = (self.window.as_ref(), self.renderer.as_mut()) else {
            return;
        };
        if window.id() != window_id {
            return;
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => renderer.resize(size),
            WindowEvent::Focused(false) => renderer.scene.input = MovementInput::default(),
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                renderer.camera.dragging = state == ElementState::Pressed;
            }
            WindowEvent::CursorMoved { position, .. } => renderer.camera.cursor_moved(position),
            WindowEvent::MouseWheel { delta, .. } => renderer.camera.zoom(delta),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let pressed = event.state == ElementState::Pressed;
                    if pressed && code == KeyCode::Escape {
                        event_loop.exit();
                    } else if pressed && code == KeyCode::KeyR {
                        renderer.reset_camera();
                    } else if pressed && code == KeyCode::Tab {
                        let direction = if renderer.scene.input.run { -1 } else { 1 };
                        renderer.scene.select_object(direction);
                        renderer.focus_selection();
                        window.set_title(&Self::title(&self.path, &renderer.scene));
                    } else if pressed && code == KeyCode::Digit0 {
                        renderer.scene.select_all_objects();
                        renderer.reset_camera();
                        window.set_title(&Self::title(&self.path, &renderer.scene));
                    } else if pressed && code == KeyCode::KeyF {
                        renderer.focus_selection();
                    } else {
                        renderer.scene.key(code, pressed);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                renderer.update();
                if let Err(error) = renderer.render() {
                    eprintln!("render error: {error}");
                    renderer.resize(renderer.size);
                }
            }
            _ => {}
        }
        window.request_redraw();
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

struct SandboxScene {
    map: TileMap,
    map_vertices: Vec<Vertex>,
    stm_vertices: Vec<Vertex>,
    stm_objects: Vec<StmObjectView>,
    selected_object: Option<usize>,
    show_collision: bool,
    show_entities: bool,
    show_point_lights: bool,
    /// Multiplier for baked lighting (VCL and lightmaps): 1.0, or 2.0 to compare an
    /// over-bright (`MODULATE2X`-style) interpretation. Toggled with `B`.
    baked_gain: f32,
    lights: Vec<PointLight>,
    player: Vec3,
    mob: Vec3,
    mob_direction: usize,
    input: MovementInput,
    elapsed: f32,
    moving: bool,
}

impl SandboxScene {
    fn new(
        map: TileMap,
        static_model: Option<&StaticModelFile>,
        textures: &MapTextures,
        baked: Option<&VertexColors>,
        map_lights: &[MapLight],
    ) -> Result<Self, String> {
        let player = closest_walkable_to_center(&map)
            .ok_or_else(|| "TTB map has no walkable tile".to_owned())?;
        let mob = farthest_walkable(&map, player).unwrap_or(player);
        let (stm_vertices, stm_objects) = static_model
            .map(|model| build_stm_vertices(model, &map, textures, baked))
            .unwrap_or_default();
        let map_vertices = build_map_vertices(&map);
        let lights = build_point_lights(map_lights, &map);
        eprintln!(
            "loaded TTB {}x{}, tile size {}, {} walkable tiles",
            map.width,
            map.height,
            map.tile_size,
            map.tiles.iter().filter(|tile| tile.is_walkable()).count()
        );
        if let Some(model) = static_model {
            let faces: usize = model
                .objects
                .iter()
                .flat_map(|object| &object.groups)
                .map(|group| group.faces.len())
                .sum();
            eprintln!(
                "loaded STM: {} visual objects, {faces} faces, {} materials",
                model.objects.len(),
                model.materials.len()
            );
        }
        Ok(Self {
            map,
            map_vertices,
            stm_vertices,
            stm_objects,
            selected_object: None,
            show_collision: false,
            show_entities: true,
            show_point_lights: true,
            baked_gain: 1.0,
            lights,
            player,
            mob,
            mob_direction: 0,
            input: MovementInput::default(),
            elapsed: 0.0,
            moving: false,
        })
    }

    fn key(&mut self, code: KeyCode, pressed: bool) {
        match code {
            KeyCode::KeyW | KeyCode::ArrowUp => self.input.forward = pressed,
            KeyCode::KeyS | KeyCode::ArrowDown => self.input.backward = pressed,
            KeyCode::KeyA | KeyCode::ArrowLeft => self.input.left = pressed,
            KeyCode::KeyD | KeyCode::ArrowRight => self.input.right = pressed,
            KeyCode::ShiftLeft | KeyCode::ShiftRight => self.input.run = pressed,
            KeyCode::KeyG if pressed => self.show_collision = !self.show_collision,
            KeyCode::KeyH if pressed => self.show_entities = !self.show_entities,
            KeyCode::KeyL if pressed => self.show_point_lights = !self.show_point_lights,
            KeyCode::KeyB if pressed => {
                self.baked_gain = if self.baked_gain > 1.5 { 1.0 } else { 2.0 };
            }
            _ => {}
        }
    }

    fn update(&mut self, delta: f32, camera_yaw: f32) {
        self.elapsed += delta;
        let mut input = Vec2::new(
            f32::from(self.input.right) - f32::from(self.input.left),
            f32::from(self.input.forward) - f32::from(self.input.backward),
        );
        self.moving = input.length_squared() > 0.0;
        if self.moving {
            input = input.normalize();
            let forward = Vec3::new(-camera_yaw.sin(), 0.0, -camera_yaw.cos());
            let right = Vec3::new(forward.z, 0.0, -forward.x);
            let direction = (right * input.x + forward * input.y).normalize_or_zero();
            let speed = if self.input.run { 6.0 } else { 3.3 };
            self.move_player(direction * speed * delta);
        }
        self.update_mob(delta);
    }

    fn move_player(&mut self, movement: Vec3) {
        let desired = self.player + movement;
        let along_x = Vec3::new(desired.x, 0.0, self.player.z);
        if self.can_stand(along_x, 0.20) {
            self.player.x = desired.x;
        }
        let along_z = Vec3::new(self.player.x, 0.0, desired.z);
        if self.can_stand(along_z, 0.20) {
            self.player.z = desired.z;
        }
    }

    fn update_mob(&mut self, delta: f32) {
        const DIRECTIONS: [Vec3; 4] = [Vec3::X, Vec3::Z, Vec3::NEG_X, Vec3::NEG_Z];
        for _ in 0..DIRECTIONS.len() {
            let desired = self.mob + DIRECTIONS[self.mob_direction] * 1.35 * delta;
            if self.can_stand(desired, 0.24) {
                self.mob = desired;
                return;
            }
            self.mob_direction = (self.mob_direction + 1) % DIRECTIONS.len();
        }
    }

    fn can_stand(&self, position: Vec3, radius: f32) -> bool {
        [
            Vec2::new(-radius, -radius),
            Vec2::new(radius, -radius),
            Vec2::new(-radius, radius),
            Vec2::new(radius, radius),
        ]
        .into_iter()
        .all(|offset| self.walkable_at(position.x + offset.x, position.z + offset.y))
    }

    fn walkable_at(&self, x: f32, z: f32) -> bool {
        let map_x = (x + self.map.width as f32 * 0.5).floor();
        let map_z = (z + self.map.height as f32 * 0.5).floor();
        map_x >= 0.0 && map_z >= 0.0 && self.map.is_walkable(map_x as u32, map_z as u32)
    }

    fn vertices(&self) -> Vec<Vertex> {
        let map_vertex_count = if self.show_collision {
            self.map_vertices.len()
        } else {
            0
        };
        let selected_vertices = self
            .selected_object
            .and_then(|index| self.stm_objects.get(index))
            .map_or(self.stm_vertices.as_slice(), |object| {
                &self.stm_vertices[object.vertices.clone()]
            });
        let entity_vertex_count = if self.show_entities { 200 } else { 0 };
        let mut vertices =
            Vec::with_capacity(selected_vertices.len() + map_vertex_count + entity_vertex_count);
        vertices.extend_from_slice(selected_vertices);
        if self.show_collision {
            vertices.extend_from_slice(&self.map_vertices);
        }
        if self.show_entities {
            let bob = if self.moving {
                (self.elapsed * 11.0).sin() * 0.035
            } else {
                0.0
            };
            add_humanoid(&mut vertices, self.player + Vec3::Y * bob);
            add_mob(
                &mut vertices,
                self.mob + Vec3::Y * ((self.elapsed * 4.0).sin() * 0.06),
            );
        }
        vertices
    }

    fn maximum_vertex_count(&self) -> usize {
        self.stm_vertices.len() + self.map_vertices.len() + 200
    }

    fn select_object(&mut self, direction: isize) {
        if self.stm_objects.is_empty() {
            self.selected_object = None;
            return;
        }
        let count = self.stm_objects.len() as isize;
        let current = self.selected_object.map_or(-1, |index| index as isize);
        self.selected_object = Some((current + direction).rem_euclid(count) as usize);
    }

    fn select_all_objects(&mut self) {
        self.selected_object = None;
    }

    fn selected_view(&self) -> Option<&StmObjectView> {
        self.selected_object
            .and_then(|index| self.stm_objects.get(index))
    }

    fn selection_label(&self) -> String {
        self.selected_view().map_or_else(
            || format!("todas as {} peças", self.stm_objects.len()),
            |object| {
                format!(
                    "peça {}/{}: {} | tipo {} | {} triângulos",
                    self.selected_object.unwrap_or_default() + 1,
                    self.stm_objects.len(),
                    object.name,
                    object.object_type,
                    object.vertices.len() / 3
                )
            },
        )
    }
}

#[derive(Debug, Clone)]
struct StmObjectView {
    name: String,
    object_type: u32,
    vertices: Range<usize>,
    center: Vec3,
    radius: f32,
}

#[derive(Default)]
struct MovementInput {
    forward: bool,
    backward: bool,
    left: bool,
    right: bool,
    run: bool,
}

fn closest_walkable_to_center(map: &TileMap) -> Option<Vec3> {
    let center = Vec2::new(map.width as f32 * 0.5, map.height as f32 * 0.5);
    map.tiles
        .iter()
        .enumerate()
        .filter(|(_, tile)| tile.is_walkable())
        .min_by(|(left, _), (right, _)| {
            tile_distance_squared(map, *left, center)
                .total_cmp(&tile_distance_squared(map, *right, center))
        })
        .map(|(index, _)| tile_center(map, index))
}

fn farthest_walkable(map: &TileMap, from: Vec3) -> Option<Vec3> {
    map.tiles
        .iter()
        .enumerate()
        .filter(|(_, tile)| tile.is_walkable())
        .max_by(|(left, _), (right, _)| {
            tile_center(map, *left)
                .distance_squared(from)
                .total_cmp(&tile_center(map, *right).distance_squared(from))
        })
        .map(|(index, _)| tile_center(map, index))
}

fn tile_distance_squared(map: &TileMap, index: usize, point: Vec2) -> f32 {
    let x = (index % map.width as usize) as f32 + 0.5;
    let z = (index / map.width as usize) as f32 + 0.5;
    Vec2::new(x, z).distance_squared(point)
}

fn tile_center(map: &TileMap, index: usize) -> Vec3 {
    let x = (index % map.width as usize) as f32 + 0.5 - map.width as f32 * 0.5;
    let z = (index / map.width as usize) as f32 + 0.5 - map.height as f32 * 0.5;
    Vec3::new(x, 0.0, z)
}

fn build_stm_vertices(
    model: &StaticModelFile,
    map: &TileMap,
    textures: &MapTextures,
    baked: Option<&VertexColors>,
) -> (Vec<Vertex>, Vec<StmObjectView>) {
    let mut minimum = Vec3::splat(f32::INFINITY);
    let mut maximum = Vec3::splat(f32::NEG_INFINITY);
    for position in model
        .objects
        .iter()
        .flat_map(|object| object.positions.iter())
        .map(|position| Vec3::from_array(*position))
        .filter(|position| valid_stm_position(*position))
    {
        minimum = minimum.min(position);
        maximum = maximum.max(position);
    }
    if !minimum.is_finite() || !maximum.is_finite() {
        return (Vec::new(), Vec::new());
    }

    let scale = 1.0 / map.tile_size as f32;
    let center = Vec2::new(map.width as f32 * 0.5, map.height as f32 * 0.5);
    let transform = |position: [f32; 3]| {
        let source = Vec3::from_array(position);
        Vec3::new(
            source.x * scale - center.x,
            source.y * scale,
            source.z * scale - center.y,
        )
    };

    let face_count: usize = model
        .objects
        .iter()
        .flat_map(|object| &object.groups)
        .map(|group| group.faces.len())
        .sum();
    let mut vertices = Vec::with_capacity(face_count * 3);
    let mut objects = Vec::with_capacity(model.objects.len());
    // `.vcl` colours follow the file order of the type 1 objects, one per vertex.
    let mut baked_base = 0_usize;
    let mut lightmapped_objects = 0_usize;
    for object in &model.objects {
        let vertex_start = vertices.len();
        let object_lightmap = if object.object_type == 3 {
            let index = lightmapped_objects;
            lightmapped_objects += 1;
            textures.lightmap_layer(index)
        } else {
            None
        };
        let object_colors = if matches!(object.object_type, 0 | 1) {
            let base = baked_base;
            baked_base += object.positions.len();
            baked.and_then(|colors| colors.slice(base, object.positions.len()))
        } else {
            None
        };
        for group in &object.groups {
            let texture_name = model
                .materials
                .get(group.material_index as usize)
                .map(|material| material.texture_name.as_str())
                .unwrap_or("missing");
            let layer = textures.layer_for_material(group.material_index);
            let color = if layer.is_some() {
                [1.0; 3]
            } else {
                material_color(texture_name, group.material_index)
            };
            let layer = layer.map_or(-1.0, |layer| layer as f32);
            for (face_index, face) in group.faces.iter().enumerate() {
                let Some(source_positions) = face
                    .iter()
                    .map(|index| object.positions.get(usize::from(*index)).copied())
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                let positions = [
                    transform(source_positions[0]),
                    transform(source_positions[1]),
                    transform(source_positions[2]),
                ];
                if !source_positions
                    .iter()
                    .all(|position| valid_stm_position(Vec3::from_array(*position)))
                {
                    continue;
                }
                let normal = (positions[1] - positions[0])
                    .cross(positions[2] - positions[0])
                    .normalize_or(Vec3::Y);
                for (corner, position) in positions.into_iter().enumerate() {
                    let uv = object
                        .texture_coordinates
                        .get(usize::from(face[corner]))
                        .copied()
                        .unwrap_or([0.0, 0.0]);
                    let vertex = Vertex::textured(position, normal, color, uv, layer);
                    vertices.push(match object_colors {
                        Some(colors) if layer >= 0.0 => {
                            let [red, green, blue, _] = colors[usize::from(face[corner])];
                            vertex.baked([red, green, blue].map(|c| f32::from(c) / 255.0))
                        }
                        _ => match (
                            object_lightmap,
                            group.lightmap_coordinates.get(face_index * 3 + corner),
                        ) {
                            (Some(lightmap), Some(uv)) if layer >= 0.0 => {
                                vertex.lightmapped(*uv, lightmap as f32)
                            }
                            _ => vertex,
                        },
                    });
                }
            }
        }
        let vertex_end = vertices.len();
        if vertex_end > vertex_start {
            let mut object_minimum = Vec3::splat(f32::INFINITY);
            let mut object_maximum = Vec3::splat(f32::NEG_INFINITY);
            for vertex in &vertices[vertex_start..vertex_end] {
                let position = Vec3::from_array(vertex.position);
                object_minimum = object_minimum.min(position);
                object_maximum = object_maximum.max(position);
            }
            let object_center = (object_minimum + object_maximum) * 0.5;
            let radius = (object_maximum - object_minimum).length() * 0.5;
            objects.push(StmObjectView {
                name: object.name.clone(),
                object_type: object.object_type,
                vertices: vertex_start..vertex_end,
                center: object_center,
                radius,
            });
        }
    }
    eprintln!(
        "STM bounds min={minimum:?} max={maximum:?}, scale={scale:.6}, GPU vertices={}",
        vertices.len()
    );
    (vertices, objects)
}

fn valid_stm_position(position: Vec3) -> bool {
    position.is_finite() && position.abs().max_element() < 1_000_000.0
}

fn material_color(texture_name: &str, material_index: u32) -> [f32; 3] {
    let hash = texture_name
        .bytes()
        .fold(2_166_136_261_u32 ^ material_index, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        });
    [
        0.28 + (hash & 0xff) as f32 / 255.0 * 0.48,
        0.28 + ((hash >> 8) & 0xff) as f32 / 255.0 * 0.48,
        0.28 + ((hash >> 16) & 0xff) as f32 / 255.0 * 0.48,
    ]
}

fn build_map_vertices(map: &TileMap) -> Vec<Vertex> {
    let mut vertices = Vec::with_capacity(map.tiles.len() * 12);
    for (index, tile) in map.tiles.iter().copied().enumerate() {
        let center = tile_center(map, index);
        let checker = ((index % map.width as usize) + (index / map.width as usize)) & 1;
        if tile.is_walkable() {
            let color = match tile.attribute {
                9 => [0.05, 0.52, 0.68],
                0 if checker == 0 => [0.16, 0.29, 0.22],
                0 => [0.13, 0.25, 0.19],
                _ => [0.56, 0.34, 0.10],
            };
            add_floor_tile(&mut vertices, center, color);
        } else {
            add_box(
                &mut vertices,
                center + Vec3::Y * 0.18,
                Vec3::new(0.48, 0.18, 0.48),
                [0.24, 0.07, 0.08],
            );
        }
    }
    vertices
}

fn add_floor_tile(vertices: &mut Vec<Vertex>, center: Vec3, color: [f32; 3]) {
    let half = 0.48;
    add_quad(
        vertices,
        [
            center + Vec3::new(-half, 0.0, -half),
            center + Vec3::new(half, 0.0, -half),
            center + Vec3::new(half, 0.0, half),
            center + Vec3::new(-half, 0.0, half),
        ],
        Vec3::Y,
        color,
    );
}

fn add_humanoid(vertices: &mut Vec<Vertex>, position: Vec3) {
    add_box(
        vertices,
        position + Vec3::new(0.0, 0.72, 0.0),
        Vec3::new(0.22, 0.34, 0.15),
        [0.12, 0.48, 0.96],
    );
    add_box(
        vertices,
        position + Vec3::new(0.0, 1.18, 0.0),
        Vec3::splat(0.17),
        [0.90, 0.66, 0.48],
    );
    add_box(
        vertices,
        position + Vec3::new(-0.11, 0.25, 0.0),
        Vec3::new(0.08, 0.25, 0.09),
        [0.04, 0.10, 0.22],
    );
    add_box(
        vertices,
        position + Vec3::new(0.11, 0.25, 0.0),
        Vec3::new(0.08, 0.25, 0.09),
        [0.04, 0.10, 0.22],
    );
}

fn add_mob(vertices: &mut Vec<Vertex>, position: Vec3) {
    add_box(
        vertices,
        position + Vec3::new(0.0, 0.48, 0.0),
        Vec3::new(0.34, 0.38, 0.34),
        [0.76, 0.12, 0.16],
    );
    add_pyramid(
        vertices,
        position + Vec3::new(0.0, 1.05, 0.0),
        0.34,
        0.42,
        [0.96, 0.38, 0.08],
    );
}

fn add_box(vertices: &mut Vec<Vertex>, center: Vec3, half: Vec3, color: [f32; 3]) {
    let p = [
        center + Vec3::new(-half.x, -half.y, -half.z),
        center + Vec3::new(half.x, -half.y, -half.z),
        center + Vec3::new(half.x, half.y, -half.z),
        center + Vec3::new(-half.x, half.y, -half.z),
        center + Vec3::new(-half.x, -half.y, half.z),
        center + Vec3::new(half.x, -half.y, half.z),
        center + Vec3::new(half.x, half.y, half.z),
        center + Vec3::new(-half.x, half.y, half.z),
    ];
    add_quad(vertices, [p[4], p[5], p[6], p[7]], Vec3::Z, color);
    add_quad(vertices, [p[1], p[0], p[3], p[2]], Vec3::NEG_Z, color);
    add_quad(vertices, [p[0], p[4], p[7], p[3]], Vec3::NEG_X, color);
    add_quad(vertices, [p[5], p[1], p[2], p[6]], Vec3::X, color);
    add_quad(vertices, [p[3], p[7], p[6], p[2]], Vec3::Y, color);
    add_quad(vertices, [p[0], p[1], p[5], p[4]], Vec3::NEG_Y, color);
}

fn add_pyramid(vertices: &mut Vec<Vertex>, center: Vec3, half: f32, height: f32, color: [f32; 3]) {
    let base_y = center.y - height * 0.5;
    let apex = center + Vec3::Y * height * 0.5;
    let base = [
        Vec3::new(center.x - half, base_y, center.z - half),
        Vec3::new(center.x + half, base_y, center.z - half),
        Vec3::new(center.x + half, base_y, center.z + half),
        Vec3::new(center.x - half, base_y, center.z + half),
    ];
    add_quad(
        vertices,
        [base[3], base[2], base[1], base[0]],
        Vec3::NEG_Y,
        color,
    );
    for edge in 0..4 {
        let a = base[edge];
        let b = base[(edge + 1) % 4];
        let normal = (b - a).cross(apex - a).normalize_or(Vec3::Y);
        vertices.extend([
            Vertex::new(a, normal, color),
            Vertex::new(b, normal, color),
            Vertex::new(apex, normal, color),
        ]);
    }
}

fn add_quad(vertices: &mut Vec<Vertex>, corners: [Vec3; 4], normal: Vec3, color: [f32; 3]) {
    for index in [0, 1, 2, 0, 2, 3] {
        vertices.push(Vertex::new(corners[index], normal, color));
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 3],
    normal: [f32; 3],
    color: [f32; 3],
    uv: [f32; 2],
    layer: f32,
    /// `1.0` when `color` is baked lighting (VCL) that replaces dynamic lighting.
    baked: f32,
    lightmap_uv: [f32; 2],
    /// Lightmap array layer, or `-1.0` when the vertex has no lightmap.
    lightmap_layer: f32,
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 8] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x3,
        3 => Float32x2,
        4 => Float32,
        5 => Float32,
        6 => Float32x2,
        7 => Float32,
    ];

    fn new(position: Vec3, normal: Vec3, color: [f32; 3]) -> Self {
        Self::textured(position, normal, color, [0.0, 0.0], -1.0)
    }

    fn textured(position: Vec3, normal: Vec3, color: [f32; 3], uv: [f32; 2], layer: f32) -> Self {
        Self {
            position: position.to_array(),
            normal: normal.to_array(),
            color,
            uv,
            layer,
            baked: 0.0,
            lightmap_uv: [0.0, 0.0],
            lightmap_layer: -1.0,
        }
    }

    fn lightmapped(mut self, uv: [f32; 2], layer: f32) -> Self {
        self.lightmap_uv = uv;
        self.lightmap_layer = layer;
        self
    }

    fn baked(mut self, color: [f32; 3]) -> Self {
        self.color = color;
        self.baked = 1.0;
        self
    }

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

const MAX_POINT_LIGHTS: usize = 256;

/// One `GX_LIGHT` in sandbox units: `position_radius` is `xyz` + radius, `color` is `rgb` + unused.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct PointLight {
    position_radius: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct LightsUniform {
    count: [u32; 4],
    lights: [PointLight; MAX_POINT_LIGHTS],
}

impl LightsUniform {
    fn new(lights: &[PointLight], enabled: bool) -> Self {
        let mut uniform = Self::zeroed();
        if enabled {
            let count = lights.len().min(MAX_POINT_LIGHTS);
            uniform.lights[..count].copy_from_slice(&lights[..count]);
            uniform.count[0] = count as u32;
        }
        uniform
    }
}

/// Converts `GX_LIGHT` records to sandbox space, using the same transform as the STM scene.
fn build_point_lights(lights: &[MapLight], map: &TileMap) -> Vec<PointLight> {
    let scale = 1.0 / map.tile_size as f32;
    if lights.len() > MAX_POINT_LIGHTS {
        eprintln!(
            "using the first {MAX_POINT_LIGHTS} of {} point lights",
            lights.len()
        );
    }
    lights
        .iter()
        .take(MAX_POINT_LIGHTS)
        .map(|light| {
            let [red, green, blue] = light.rgb();
            PointLight {
                position_radius: [
                    light.position[0] * scale - map.width as f32 * 0.5,
                    light.position[1] * scale,
                    light.position[2] * scale - map.height as f32 * 0.5,
                    light.radius * scale,
                ],
                color: [red, green, blue, 0.0],
            }
        })
        .collect()
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct CameraUniform {
    view_projection: [[f32; 4]; 4],
    light_direction: [f32; 4],
}

struct Camera {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    dragging: bool,
    last_cursor: Option<PhysicalPosition<f64>>,
}

/// Debug override for repeatable screenshots: `CORUM_CAMERA=yaw,pitch,distance`
/// (radians, radians, tiles). Also used when the camera is reset with `R`.
fn debug_camera() -> Option<(f32, f32, f32)> {
    let value = env::var("CORUM_CAMERA").ok()?;
    let parts: Vec<f32> = value
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    let [yaw, pitch, distance] = parts[..] else {
        return None;
    };
    Some((yaw, pitch.clamp(0.08, 1.48), distance.clamp(1.0, 120.0)))
}

impl Camera {
    fn new(target: Vec3) -> Self {
        let (yaw, pitch, distance) = debug_camera().unwrap_or((0.75, 0.68, 12.0));
        Self {
            target,
            yaw,
            pitch,
            distance,
            dragging: false,
            last_cursor: None,
        }
    }

    fn cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if self.dragging
            && let Some(previous) = self.last_cursor
        {
            self.yaw -= (position.x - previous.x) as f32 * 0.008;
            self.pitch = (self.pitch + (position.y - previous.y) as f32 * 0.008).clamp(0.08, 1.48);
        }
        self.last_cursor = Some(position);
    }

    fn zoom(&mut self, delta: MouseScrollDelta) {
        let amount = match delta {
            MouseScrollDelta::LineDelta(_, value) => value,
            MouseScrollDelta::PixelDelta(value) => value.y as f32 * 0.02,
        };
        self.distance = (self.distance * (-amount * 0.12).exp()).clamp(1.0, 120.0);
    }

    fn uniform(&self, aspect: f32) -> CameraUniform {
        let horizontal = self.pitch.cos();
        let eye = self.target
            + Vec3::new(
                horizontal * self.yaw.sin(),
                self.pitch.sin(),
                horizontal * self.yaw.cos(),
            ) * self.distance;
        let view = glam::camera::rh::view::look_at_mat4(eye, self.target, Vec3::Y);
        let projection = glam::camera::rh::proj::directx::perspective(
            45_f32.to_radians(),
            aspect.max(0.01),
            0.01,
            500.0,
        );
        CameraUniform {
            view_projection: (projection * view).to_cols_array_2d(),
            light_direction: [-0.35, 0.75, 0.55, 0.0],
        }
    }
}

struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    pipeline: wgpu::RenderPipeline,
    texture_bind_group: wgpu::BindGroup,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: usize,
    vertex_count: u32,
    camera_buffer: wgpu::Buffer,
    lights_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    depth: DepthTarget,
    camera: Camera,
    scene: SandboxScene,
    previous_frame: Instant,
}

impl Renderer {
    async fn new(
        window: Arc<Window>,
        scene: SandboxScene,
        textures: MapTextures,
    ) -> Result<Self, String> {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance
            .create_surface(window)
            .map_err(|error| error.to_string())?;
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .map_err(|error| error.to_string())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("corum sandbox device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|error| error.to_string())?;
        let width = size.width.max(1);
        let height = size.height.max(1);
        let mut config = surface
            .get_default_config(&adapter, width, height)
            .ok_or_else(|| "the selected GPU cannot present to this window".to_owned())?;
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // Direct3D 8 blends raw colour values (no sRGB decode/encode), so baked lighting is
        // modulated in that same "gamma space" to match the original's look.
        let gamma_format = config.format.remove_srgb_suffix();
        if gamma_format != config.format
            && surface
                .get_capabilities(&adapter)
                .formats
                .contains(&gamma_format)
        {
            config.format = gamma_format;
        }
        surface.configure(&device, &config);

        let camera = Camera::new(scene.player + Vec3::Y * 0.7);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sandbox camera uniform"),
            contents: bytemuck::bytes_of(&camera.uniform(width as f32 / height as f32)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sandbox point lights"),
            contents: bytemuck::bytes_of(&LightsUniform::new(
                &scene.lights,
                scene.show_point_lights,
            )),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sandbox camera layout"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                uniform_entry(1, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sandbox camera bind group"),
            layout: &camera_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: camera_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: lights_buffer.as_entire_binding(),
                },
            ],
        });
        let (texture_layout, texture_bind_group) = textures.upload(&device, &queue);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sandbox shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../sandbox_shader.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sandbox pipeline layout"),
            bind_group_layouts: &[Some(&camera_layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sandbox pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[Some(Vertex::layout())],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let vertices = scene.vertices();
        let vertex_capacity = scene.maximum_vertex_count();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sandbox vertices"),
            size: (vertex_capacity * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        let depth = DepthTarget::new(&device, width, height);
        eprintln!("uploaded {} triangles", vertices.len() / 3);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            pipeline,
            texture_bind_group,
            vertex_buffer,
            vertex_capacity,
            vertex_count: vertices.len() as u32,
            camera_buffer,
            lights_buffer,
            camera_bind_group,
            depth,
            camera,
            scene,
            previous_frame: Instant::now(),
        })
    }

    fn reset_camera(&mut self) {
        if self.scene.selected_view().is_some() {
            self.focus_selection();
        } else {
            self.camera = Camera::new(self.scene.player + Vec3::Y * 0.7);
        }
    }

    fn focus_selection(&mut self) {
        let Some(object) = self.scene.selected_view() else {
            return;
        };
        self.camera.target = object.center;
        self.camera.distance = (object.radius * 2.2).clamp(2.5, 120.0);
    }

    fn resize(&mut self, size: PhysicalSize<u32>) {
        self.size = size;
        if size.width == 0 || size.height == 0 {
            return;
        }
        self.config.width = size.width;
        self.config.height = size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = DepthTarget::new(&self.device, size.width, size.height);
    }

    fn update(&mut self) {
        let now = Instant::now();
        let delta = (now - self.previous_frame).as_secs_f32().min(0.05);
        self.previous_frame = now;
        self.scene.update(delta, self.camera.yaw);
        self.camera.target = self.scene.player + Vec3::Y * 0.7;
        let vertices = self.scene.vertices();
        if vertices.len() <= self.vertex_capacity {
            self.queue
                .write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            self.vertex_count = vertices.len() as u32;
        }
        if self.config.height != 0 {
            let mut uniform = self
                .camera
                .uniform(self.config.width as f32 / self.config.height as f32);
            uniform.light_direction[3] = self.scene.baked_gain;
            self.queue
                .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
            let lights = LightsUniform::new(&self.scene.lights, self.scene.show_point_lights);
            self.queue
                .write_buffer(&self.lights_buffer, 0, bytemuck::bytes_of(&lights));
        }
    }

    fn render(&mut self) -> Result<(), String> {
        if self.size.width == 0 || self.size.height == 0 {
            return Ok(());
        }
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                self.surface.configure(&self.device, &self.config);
                frame
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                return Err("surface validation error".to_owned());
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("sandbox frame encoder"),
            });
        {
            let color_attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.143,
                        g: 0.179,
                        b: 0.235,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sandbox render pass"),
                color_attachments: &color_attachments,
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            pass.set_bind_group(1, &self.texture_bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(0..self.vertex_count, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
        Ok(())
    }
}

struct DepthTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl DepthTarget {
    fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("sandbox depth target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
        }
    }
}

/// The texture packages a map can draw from.
struct TexturePackages {
    dds: Option<PakArchive>,
    tif: Option<PakArchive>,
}

/// Map textures resolved from the `Map_dds` and `Map_tif` packages, one array layer per texture.
#[derive(Default)]
struct MapTextures {
    layer_size: u32,
    images: Vec<DecodedImage>,
    material_layers: Vec<Option<u32>>,
    lightmap_size: u32,
    lightmaps: Vec<DecodedImage>,
    /// Lightmap layer per lightmapped (type 3) object, in scene order.
    lightmap_layers: Vec<Option<u32>>,
}

impl MapTextures {
    /// Resolves every STM material to a DDS texture. The package lives in the sibling
    /// `Map_dds` folder of the `Map` directory that holds the `.ttb`; missing textures
    /// simply fall back to the flat per-material colour.
    fn load(model: &StaticModelFile, ttb_path: &Path) -> Self {
        let packages = TexturePackages {
            dds: open_data_package(ttb_path, "Map_dds")
                .map_err(|error| eprintln!("DDS textures unavailable: {error}"))
                .ok(),
            tif: open_data_package(ttb_path, "Map_tif")
                .map_err(|error| eprintln!("TIFF textures unavailable: {error}"))
                .ok(),
        };

        let mut textures = Self::default();
        let mut layer_by_name: Vec<(String, Option<u32>)> = Vec::new();
        for material in &model.materials {
            let key = material.texture_name.to_ascii_lowercase();
            if let Some((_, layer)) = layer_by_name.iter().find(|(name, _)| *name == key) {
                textures.material_layers.push(*layer);
                continue;
            }
            let layer = textures.decode(&packages, &material.texture_name);
            layer_by_name.push((key, layer));
            textures.material_layers.push(layer);
        }
        textures.layer_size = textures
            .images
            .iter()
            .map(|image| image.width.max(image.height))
            .max()
            .unwrap_or(1);
        eprintln!(
            "loaded {} unique textures for {} materials",
            textures.images.len(),
            model.materials.len()
        );
        textures
    }

    /// Materials name a `.tga`, but the packages hold the same texture as `.dds` (96% of the
    /// 196 packed maps' materials) or, for the rest, as an uncompressed `.tif`.
    fn decode(&mut self, packages: &TexturePackages, texture_name: &str) -> Option<u32> {
        let stem = texture_name
            .rsplit_once('.')
            .map_or(texture_name, |(stem, _)| stem);
        let dds = format!("{stem}.dds");
        let tif = format!("{stem}.tif");
        let read = |archive: &Option<PakArchive>, entry: &str| {
            archive
                .as_ref()
                .and_then(|archive| archive.read_entry(entry).ok())
        };
        let decoded = match read(&packages.dds, &dds) {
            Some(bytes) => {
                DecodedImage::from_dds(&bytes).map_err(|error| format!("texture '{dds}': {error}"))
            }
            None => {
                let bytes = read(&packages.tif, &tif)?;
                DecodedImage::from_tiff(&bytes).map_err(|error| format!("texture '{tif}': {error}"))
            }
        };
        match decoded {
            Ok(image) => {
                self.images.push(image);
                Some((self.images.len() - 1) as u32)
            }
            Err(message) => {
                eprintln!("{message}");
                None
            }
        }
    }

    /// Pairs the k-th type 3 object with the k-th lightmap record. The object repeats its
    /// record's header in its trailing data, so a mismatching pair is skipped instead of
    /// showing another object's lighting.
    fn attach_lightmaps(&mut self, model: &StaticModelFile, file: &LightmapFile) {
        let objects = model
            .objects
            .iter()
            .filter(|object| object.object_type == 3);
        for (index, object) in objects.enumerate() {
            let layer = file.maps.get(index).and_then(|map| {
                let agrees = object.lightmap.is_some_and(|descriptor| {
                    descriptor.first_field == map.first_field
                        && descriptor.width == map.width
                        && descriptor.height == map.height
                });
                if !agrees {
                    eprintln!("lightmap #{index} does not match object '{}'", object.name);
                    return None;
                }
                self.lightmaps.push(DecodedImage {
                    width: map.width,
                    height: map.height,
                    rgba: map.rgba.clone(),
                });
                Some((self.lightmaps.len() - 1) as u32)
            });
            self.lightmap_layers.push(layer);
        }
        self.lightmap_size = self
            .lightmaps
            .iter()
            .map(|image| image.width.max(image.height))
            .max()
            .unwrap_or(1);
        eprintln!(
            "loaded LM: {} lightmaps for {} lightmapped objects",
            self.lightmaps.len(),
            self.lightmap_layers.len()
        );
    }

    fn lightmap_layer(&self, object_index: usize) -> Option<u32> {
        self.lightmap_layers.get(object_index).copied().flatten()
    }

    fn layer_for_material(&self, material_index: u32) -> Option<u32> {
        self.material_layers
            .get(material_index as usize)
            .copied()
            .flatten()
    }

    fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> (wgpu::BindGroupLayout, wgpu::BindGroup) {
        let size = self.layer_size.max(1);
        let layers = self.images.len().max(1) as u32;
        let mip_levels = size.ilog2() + 1;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("map texture array"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers,
            },
            mip_level_count: mip_levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let placeholder = DecodedImage {
            width: 1,
            height: 1,
            rgba: vec![255; 4],
        };
        for layer in 0..layers {
            let image = self.images.get(layer as usize).unwrap_or(&placeholder);
            let mut level = resample(image, size);
            let mut level_size = size;
            for mip in 0..mip_levels {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: mip,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &level,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(level_size * 4),
                        rows_per_image: Some(level_size),
                    },
                    wgpu::Extent3d {
                        width: level_size,
                        height: level_size,
                        depth_or_array_layers: 1,
                    },
                );
                if level_size > 1 {
                    level = downsample(&level, level_size);
                    level_size /= 2;
                }
            }
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        // Lightmaps hold raw light intensity, so they are sampled without sRGB decoding.
        let lightmap_size = self.lightmap_size.max(1);
        let lightmap_layers = self.lightmaps.len().max(1) as u32;
        let lightmap_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("map lightmap array"),
            size: wgpu::Extent3d {
                width: lightmap_size,
                height: lightmap_size,
                depth_or_array_layers: lightmap_layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for layer in 0..lightmap_layers {
            let image = self.lightmaps.get(layer as usize).unwrap_or(&placeholder);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &lightmap_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                &resample_bilinear(image, lightmap_size),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(lightmap_size * 4),
                    rows_per_image: Some(lightmap_size),
                },
                wgpu::Extent3d {
                    width: lightmap_size,
                    height: lightmap_size,
                    depth_or_array_layers: 1,
                },
            );
        }
        let lightmap_view = lightmap_texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let lightmap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("map lightmap sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("map texture sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("map texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("map texture bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&lightmap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&lightmap_sampler),
                },
            ],
        });
        (layout, bind_group)
    }
}

/// Nearest-neighbour resize to a square layer; exact for the power-of-two sizes the map uses.
fn resample(image: &DecodedImage, size: u32) -> Vec<u8> {
    let mut output = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let source_y = (u64::from(y) * u64::from(image.height) / u64::from(size)) as usize;
        for x in 0..size {
            let source_x = (u64::from(x) * u64::from(image.width) / u64::from(size)) as usize;
            let offset = (source_y * image.width as usize + source_x) * 4;
            output.extend_from_slice(&image.rgba[offset..offset + 4]);
        }
    }
    output
}

/// Bilinear resize to a square layer, so small lightmaps look like the smoothly filtered
/// textures the original renderer would have shown.
fn resample_bilinear(image: &DecodedImage, size: u32) -> Vec<u8> {
    let (width, height) = (image.width as usize, image.height as usize);
    let mut output = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        let source_y = ((y as f32 + 0.5) * height as f32 / size as f32 - 0.5).max(0.0);
        let y0 = (source_y.floor() as usize).min(height - 1);
        let y1 = (y0 + 1).min(height - 1);
        let fy = source_y - y0 as f32;
        for x in 0..size {
            let source_x = ((x as f32 + 0.5) * width as f32 / size as f32 - 0.5).max(0.0);
            let x0 = (source_x.floor() as usize).min(width - 1);
            let x1 = (x0 + 1).min(width - 1);
            let fx = source_x - x0 as f32;
            for channel in 0..4 {
                let texel =
                    |px: usize, py: usize| f32::from(image.rgba[(py * width + px) * 4 + channel]);
                let top = texel(x0, y0) * (1.0 - fx) + texel(x1, y0) * fx;
                let bottom = texel(x0, y1) * (1.0 - fx) + texel(x1, y1) * fx;
                output.push((top * (1.0 - fy) + bottom * fy + 0.5) as u8);
            }
        }
    }
    output
}

fn downsample(level: &[u8], size: u32) -> Vec<u8> {
    let half = (size / 2).max(1) as usize;
    let size = size as usize;
    let mut output = Vec::with_capacity(half * half * 4);
    for y in 0..half {
        for x in 0..half {
            for channel in 0..4 {
                let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                    .into_iter()
                    .map(|(dx, dy)| {
                        u32::from(level[((y * 2 + dy) * size + x * 2 + dx) * 4 + channel])
                    })
                    .sum();
                output.push(((sum + 2) / 4) as u8);
            }
        }
    }
    output
}

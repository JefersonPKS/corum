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
use corum_assets::stm::StaticModelFile;
use corum_assets::ttb::TileMap;
use glam::{Vec2, Vec3};
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const DEFAULT_MAP: &str = r"D:\Games\CorumOnline\Data\Map\1100.ttb";
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
    let textures = static_model
        .as_ref()
        .map(|model| MapTextures::load(model, &path))
        .unwrap_or_default();
    let scene = SandboxScene::new(map, static_model.as_ref(), &textures)?;
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let mut application = SandboxApplication::new(path, scene, textures);
    event_loop
        .run_app(&mut application)
        .map_err(|error| error.to_string())
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
            "Corum Map Viewer — {} — {} — Tab: peça | Shift+Tab: anterior | 0: tudo | F: foco | G: colisão | H: entidades",
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
    ) -> Result<Self, String> {
        let player = closest_walkable_to_center(&map)
            .ok_or_else(|| "TTB map has no walkable tile".to_owned())?;
        let mob = farthest_walkable(&map, player).unwrap_or(player);
        let (stm_vertices, stm_objects) = static_model
            .map(|model| build_stm_vertices(model, &map, textures))
            .unwrap_or_default();
        let map_vertices = build_map_vertices(&map);
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
    for object in &model.objects {
        let vertex_start = vertices.len();
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
            for face in &group.faces {
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
                    vertices.push(Vertex::textured(position, normal, color, uv, layer));
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
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x3,
        3 => Float32x2,
        4 => Float32,
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
        }
    }

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
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

impl Camera {
    fn new(target: Vec3) -> Self {
        Self {
            target,
            yaw: 0.75,
            pitch: 0.68,
            distance: 12.0,
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
        surface.configure(&device, &config);

        let camera = Camera::new(scene.player + Vec3::Y * 0.7);
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sandbox camera uniform"),
            contents: bytemuck::bytes_of(&camera.uniform(width as f32 / height as f32)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sandbox camera layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sandbox camera bind group"),
            layout: &camera_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
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
            let uniform = self
                .camera
                .uniform(self.config.width as f32 / self.config.height as f32);
            self.queue
                .write_buffer(&self.camera_buffer, 0, bytemuck::bytes_of(&uniform));
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
                        r: 0.018,
                        g: 0.027,
                        b: 0.045,
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

/// Map textures resolved from the `Map_dds` package, one array layer per unique texture.
#[derive(Default)]
struct MapTextures {
    layer_size: u32,
    images: Vec<DecodedImage>,
    material_layers: Vec<Option<u32>>,
}

impl MapTextures {
    /// Resolves every STM material to a DDS texture. The package lives in the sibling
    /// `Map_dds` folder of the `Map` directory that holds the `.ttb`; missing textures
    /// simply fall back to the flat per-material colour.
    fn load(model: &StaticModelFile, ttb_path: &Path) -> Self {
        let package = ttb_path
            .canonicalize()
            .ok()
            .as_deref()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map(|data| data.join("Map_dds").join("Map_dds.pak"));
        let archive = package
            .as_ref()
            .and_then(|path| match PakArchive::open(path) {
                Ok(archive) => Some(archive),
                Err(error) => {
                    eprintln!("textures unavailable ({}): {error}", path.display());
                    None
                }
            });

        let mut textures = Self::default();
        let mut layer_by_name: Vec<(String, Option<u32>)> = Vec::new();
        for material in &model.materials {
            let key = material.texture_name.to_ascii_lowercase();
            if let Some((_, layer)) = layer_by_name.iter().find(|(name, _)| *name == key) {
                textures.material_layers.push(*layer);
                continue;
            }
            let layer = archive
                .as_ref()
                .and_then(|archive| textures.decode(archive, &material.texture_name));
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

    fn decode(&mut self, archive: &PakArchive, texture_name: &str) -> Option<u32> {
        let stem = texture_name
            .rsplit_once('.')
            .map_or(texture_name, |(stem, _)| stem);
        let entry = format!("{stem}.dds");
        let bytes = archive.read_entry(&entry).ok()?;
        match DecodedImage::from_dds(&bytes) {
            Ok(image) => {
                self.images.push(image);
                Some((self.images.len() - 1) as u32)
            }
            Err(error) => {
                eprintln!("texture '{entry}': {error}");
                None
            }
        }
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
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
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

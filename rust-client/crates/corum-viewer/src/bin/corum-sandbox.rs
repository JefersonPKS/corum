#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use combat::{
    HURT_SECONDS, MOB_ATTACK_COOLDOWN, MOB_CHASE_SPEED, MOB_DAMAGE, MOB_MAX_HP, MOB_REACH,
    MOB_RESPAWN_SECONDS, MobMode, PLAYER_MAX_HP, PLAYER_REACH, PLAYER_RESPAWN_SECONDS, Vitals,
    item_type, mob_decision, mob_motion, mob_slot, player_damage_roll, player_motion, player_slot,
    ray_hits_sphere,
};
use corum_assets::PakArchive;
use corum_assets::cdt::Cdt;
use corum_assets::chr::ChrManifest;
use corum_assets::dds::DecodedImage;
use corum_assets::items::ItemCatalog;
use corum_assets::lightmap::LightmapFile;
use corum_assets::map_script::{MapLight, MapObject, MapScript};
use corum_assets::model::{Matrix, ModelFile};
use corum_assets::motion::MotionFile;
use corum_assets::navigation;
use corum_assets::pose::{Skeleton, transform_point, transform_vector};
use corum_assets::stm::StaticModelFile;
use corum_assets::ttb::TileMap;
use corum_assets::ui::{UiCatalog, UiDesktop};
use corum_assets::vcl::VertexColors;
use glam::{Mat4, Vec2, Vec3};
use ui_gpu::UiGpu;
use wgpu::util::DeviceExt;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

#[path = "../combat.rs"]
mod combat;
#[path = "../ui_gpu.rs"]
mod ui_gpu;

const DEFAULT_MAP: &str = r"D:\Games\CorumOnline\Data\Map\1100.ttb";
const DEFAULT_DATA_DIRECTORY: &str = r"D:\Games\CorumOnline\Data";
/// Default actors as `Package/entry`; override with `CORUM_PLAYER` and `CORUM_MOB`.
const DEFAULT_PLAYER_MODEL: &str = "Npc/npc007.mod";
const DEFAULT_MOB_MODEL: &str = "Monster/m00010.mod";
/// Blend classes stored in [`Vertex::blend`]; each has its own pipeline.
const BLEND_OPAQUE: f32 = 0.0;
const BLEND_ALPHA: f32 = 1.0;
const BLEND_ADDITIVE: f32 = 2.0;
/// `.MOD` material flag for additive blending: set on fire (`RD_EFFECT_FIRE`), window glows
/// (`Two_Window_Light1`), waterfalls and effect quads, and never on ordinary surfaces.
const MATERIAL_ADDITIVE: u32 = 0x4;
/// A texture is translucent (water, glass) when more than this fraction of its texels have a
/// partial alpha. Measured on the 399 TIFFs with alpha: water and glass sit at 0.9..1.0, hard
/// cut-outs (leaves, grass, fences) below 0.3.
const TRANSLUCENT_PARTIAL_FRACTION: f32 = 0.6;
/// An opaque texture with more than this fraction of near-black texels is an effect drawn on a
/// black background (fire, smoke, lightning, glows): added to the frame, black adds nothing.
/// Measured on the 71 opaque textures above 50% black: they are almost all effects (`fireb_02`
/// is 0.75, `rd_effect_fire` 0.80, `lighthouse01` 0.52 is a real surface).
const BLACK_KEYED_FRACTION: f32 = 0.7;
/// Rotation (radians) added to the movement heading so a model's front faces where it walks.
const ACTOR_YAW_OFFSET: f32 = 0.0;

/// The turn that points a model's front along its movement. The sandbox turns an actor by the
/// heading `atan2(x, z)`, which assumes the model looks toward +Z; the models of the `Character`
/// and `Monster` packages look the other way (they walked backwards without this). `Npc` models
/// were not checked.
fn facing_offset(package: &str) -> f32 {
    if package.eq_ignore_ascii_case("character") || package.eq_ignore_ascii_case("monster") {
        std::f32::consts::PI
    } else {
        0.0
    }
}
/// Pixels the cursor may move between press and release for the click to still be a move order
/// (a longer drag orbits the camera instead).
const CLICK_DRAG_LIMIT: f32 = 5.0;
/// Height of the monster's body for picking it with a click, and where its life bar floats.
const MOB_BODY_HEIGHT: f32 = 1.8;
const PLAYER_BAR_HEIGHT: f32 = 2.05;
const MOB_BAR_HEIGHT: f32 = 2.25;

/// Interface window a key opens or closes. Play mode uses the original client's keys (`KeyConfig.ini`:
/// `T` inventory, `A` character, `S` skills, `O` options); the development mode keeps `WASD` for
/// moving and uses `I`, `C`, `K` and `P` instead.
fn ui_hotkey(code: KeyCode, game: bool) -> Option<&'static str> {
    match (game, code) {
        (true, KeyCode::KeyT) | (false, KeyCode::KeyI) => Some("ITEM"),
        (true, KeyCode::KeyA) | (false, KeyCode::KeyC) => Some("CHAR"),
        (true, KeyCode::KeyS) | (false, KeyCode::KeyK) => Some("SKILL"),
        (true, KeyCode::KeyO) | (false, KeyCode::KeyP) => Some("GAMEMENU"),
        _ => None,
    }
}
/// `CORUM_GAME=1`: play mode (mouse to walk, the original hotkeys open the windows).
fn game_mode() -> bool {
    env::var("CORUM_GAME").is_ok_and(|value| value != "0")
}
/// Radius, in tiles, of the player's body when walking and planning routes.
const PLAYER_RADIUS: f32 = 0.20;
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
    // The `.stm` sits next to the `.ttb` for a few maps; the rest live in `Map_stm.pak`.
    let stm_bytes = if stm_path.is_file() {
        Some(
            fs::read(&stm_path)
                .map_err(|error| format!("could not read '{}': {error}", stm_path.display()))?,
        )
    } else {
        stm_path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| {
                open_data_package(&path, "Map_stm")
                    .ok()
                    .and_then(|archive| archive.read_entry(name).ok())
            })
    };
    let static_model = match stm_bytes {
        Some(bytes) => Some(StaticModelFile::parse(&bytes).map_err(|error| error.to_string())?),
        None => {
            eprintln!(
                "STM not found for '{}' (next to the map or in Map_stm.pak); showing collision only",
                stm_path.display()
            );
            None
        }
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
    let tile_size = map.tile_size as f32;
    let mut scene = SandboxScene::new(
        map,
        static_model.as_ref(),
        &textures,
        baked.as_ref(),
        &lights,
    )?;
    scene.props = load_props(&path, &scene.map);
    // `CORUM_PLAYER` names the body model directly; otherwise an outfit (`CORUM_CLASS`,
    // `CORUM_ARMOR`, ...) builds the character, and with neither the default NPC stands in.
    let mut weapon_type = 0;
    let mut player_cdt = None;
    match Outfit::from_env().filter(|_| env::var_os("CORUM_PLAYER").is_none()) {
        Some(outfit) => {
            let (body, parts) = outfit.build(&path, tile_size);
            scene.player_model = body;
            scene.attachments = parts;
            weapon_type = item_type(outfit.right);
            player_cdt = load_cdt(&path, &format!("pm{:02}000", outfit.class));
        }
        None => {
            scene.player_model = load_actor(&path, "CORUM_PLAYER", DEFAULT_PLAYER_MODEL, tile_size);
        }
    }
    scene.mob_model = load_actor(&path, "CORUM_MOB", DEFAULT_MOB_MODEL, tile_size);
    // The monster's own `.cdt` (key frames of its swing), named after its model.
    let mob_spec = env::var("CORUM_MOB").unwrap_or_else(|_| DEFAULT_MOB_MODEL.to_owned());
    let mob_stem = mob_spec
        .rsplit(['/', '\\'])
        .next()
        .and_then(|name| name.rsplit_once('.').map(|(stem, _)| stem))
        .unwrap_or_default()
        .to_owned();
    let mob_cdt = load_cdt(&path, &mob_stem);
    scene.setup_combat(weapon_type, player_cdt.as_ref(), mob_cdt.as_ref());
    scene.pending_ui = load_interface(&path);
    // Debug: `CORUM_AUTOFIGHT=1` orders the attack on the monster right away (repeatable checks).
    if env::var_os("CORUM_AUTOFIGHT").is_some() {
        scene.attack_mob();
    }
    // Debug: `CORUM_PLAYER_AT=x,z` starts the player at those map-script coordinates, to point
    // the camera at a specific spot in repeatable screenshots.
    if let Some([x, z]) = env::var("CORUM_PLAYER_AT").ok().and_then(|value| {
        let parts: Vec<f32> = value
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        <[f32; 2]>::try_from(parts).ok()
    }) {
        let units = 1.0 / tile_size;
        scene.player = Vec3::new(
            x * units - scene.map.width as f32 * 0.5,
            0.0,
            z * units - scene.map.height as f32 * 0.5,
        );
    }
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let mut application = SandboxApplication::new(path, scene, textures);
    event_loop
        .run_app(&mut application)
        .map_err(|error| error.to_string())
}

/// `Data\\Cdt\\<stem>.cdt`: the key frames of each motion of a character or monster.
fn load_cdt(ttb_path: &Path, stem: &str) -> Option<Cdt> {
    data_directories(ttb_path)
        .into_iter()
        .find_map(|directory| {
            let bytes = fs::read(directory.join("Cdt").join(format!("{stem}.cdt"))).ok()?;
            Cdt::parse(&bytes).ok()
        })
}

/// The original interface: its tables from `Data\Manager` and the images of the `UI` package.
/// `CORUM_UI=off` turns it off; a missing table or package only disables the windows.
fn load_interface(ttb_path: &Path) -> Option<(UiDesktop, Option<PakArchive>)> {
    if env::var("CORUM_UI").is_ok_and(|value| value.eq_ignore_ascii_case("off")) {
        return None;
    }
    let directory = data_directories(ttb_path).into_iter().find(|directory| {
        directory
            .join("Manager")
            .join("InterfaceComponentInfo.cdb")
            .is_file()
    })?;
    let catalog = UiCatalog::load(&directory.join("Manager"))
        .map_err(|error| eprintln!("interface unavailable: {error}"))
        .ok()?;
    let archive = open_data_package(ttb_path, "UI")
        .map_err(|error| eprintln!("interface images unavailable: {error}"))
        .ok();
    eprintln!("loaded interface: {} windows", catalog.frames().count());
    Some((UiDesktop::new(catalog), archive))
}

/// Loads the actor named by `variable` (or `default`), given as `Package/entry`. A model that
/// cannot be found falls back to the placeholder boxes.
/// `CORUM_PLAYER_ANIM` / `CORUM_MOB_ANIM` = `idle,moving` picks the `.chr` motion slots
/// (0-based; `-` for none) instead of the package defaults.
fn motion_slots_override(package: &str, variable: &str) -> (Option<usize>, Option<usize>) {
    let defaults = default_motion_slots(package);
    let Ok(value) = env::var(variable) else {
        return defaults;
    };
    let mut parts = value
        .split(',')
        .map(|part| part.trim().parse::<usize>().ok());
    (
        parts.next().unwrap_or(defaults.0),
        parts.next().unwrap_or(defaults.1),
    )
}

fn load_actor(
    ttb_path: &Path,
    variable: &str,
    default: &str,
    tile_size: f32,
) -> Option<ActorModel> {
    let spec = env::var(variable).unwrap_or_else(|_| default.to_owned());
    if spec.eq_ignore_ascii_case("none") {
        return None;
    }
    let Some((package, entry)) = spec.split_once('/') else {
        eprintln!("{variable}='{spec}' must look like Package/entry.mod");
        return None;
    };
    match ActorModel::load(
        ttb_path,
        package,
        entry,
        tile_size,
        &format!("{variable}_ANIM"),
    ) {
        Ok(model) => Some(model),
        Err(error) => {
            eprintln!("actor {spec} unavailable: {error}");
            None
        }
    }
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
        if game_mode() {
            return "Corum Online (Rust) — clique: andar ou atacar o monstro | T: inventário | A: personagem | S: habilidades | O: opções | Esc: fechar janela ou sair".to_owned();
        }
        format!(
            "Corum Map Viewer — {} — {} — clique: andar | Tab: peça | Shift+Tab: anterior | 0: tudo | F: foco | G: colisão | H: entidades | L: luzes | B: brilho | O: objetos",
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
                let pressed = state == ElementState::Pressed;
                let window_size = (renderer.config.width as f32, renderer.config.height as f32);
                let cursor = renderer
                    .camera
                    .last_cursor
                    .map_or((0.0, 0.0), |cursor| (cursor.x, cursor.y));
                if pressed {
                    renderer.ui_captured = renderer.ui.as_mut().is_some_and(|ui| {
                        let point = ui_gpu::to_screen(window_size, cursor);
                        ui.desktop.press(point) != corum_assets::ui::Press::World
                    });
                    renderer.camera.drag_distance = 0.0;
                } else if let Some(ui) = &mut renderer.ui {
                    ui.desktop.release();
                }
                renderer.camera.dragging = pressed && !renderer.ui_captured;
                // A click that did not turn into a drag is a move order (as in the original client),
                // unless it was for an interface window.
                if !pressed
                    && !renderer.ui_captured
                    && renderer.camera.drag_distance < CLICK_DRAG_LIMIT
                {
                    renderer.click_to_move();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                renderer.camera.cursor_moved(position);
                let window_size = (renderer.config.width as f32, renderer.config.height as f32);
                if let Some(ui) = &mut renderer.ui {
                    ui.desktop
                        .motion(ui_gpu::to_screen(window_size, (position.x, position.y)));
                }
            }
            WindowEvent::MouseWheel { delta, .. } => renderer.camera.zoom(delta),
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    let pressed = event.state == ElementState::Pressed;
                    let hotkey = ui_hotkey(code, renderer.game_mode).filter(|_| pressed);
                    if pressed && code == KeyCode::Escape {
                        // Escape closes the window on top; with none open it leaves the game.
                        let closed = renderer
                            .ui
                            .as_mut()
                            .is_some_and(|ui| ui.desktop.close_top());
                        if !closed {
                            event_loop.exit();
                        }
                    } else if let Some(name) = hotkey
                        && renderer.ui.is_some()
                    {
                        if let Some(ui) = &mut renderer.ui
                            && let Some(id) = ui.desktop.catalog().window_named(name)
                        {
                            ui.desktop.toggle(id);
                        }
                    } else if renderer.game_mode
                        && matches!(
                            code,
                            KeyCode::KeyW | KeyCode::KeyA | KeyCode::KeyS | KeyCode::KeyD
                        )
                    {
                        // In play mode the letters belong to the original hotkeys; the arrows
                        // and the mouse move the character.
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum PlayerState {
    Free,
    Attacking,
    Hurt,
    Dead,
}

/// Offline combat state (toy numbers, see `combat.rs`); the timings come from the motions.
struct Fight {
    player_life: Vitals,
    player_state: PlayerState,
    player_clock: f32,
    player_start: Vec3,
    /// Weapon type of the right hand (`id / 200 + 1`), which picks the character's motion set.
    item_type: u16,
    /// A click on the monster: keep walking to it and swinging until it falls.
    attack_target: bool,
    replan: f32,
    blows: u32,
    player_hit_done: bool,
    player_attack_seconds: f32,
    player_hit_seconds: f32,
    player_hurt_seconds: f32,
    mob_life: Vitals,
    mob_mode: MobMode,
    mob_clock: f32,
    mob_spawn: Vec3,
    mob_hit_done: bool,
    mob_attack_seconds: f32,
    mob_hit_seconds: f32,
    mob_hurt_seconds: f32,
}

impl Fight {
    fn new(player: Vec3, mob: Vec3) -> Self {
        Self {
            player_life: Vitals::full(PLAYER_MAX_HP),
            player_state: PlayerState::Free,
            player_clock: 0.0,
            player_start: player,
            item_type: 0,
            attack_target: false,
            replan: 0.0,
            blows: 0,
            player_hit_done: false,
            player_attack_seconds: 0.9,
            player_hit_seconds: 0.45,
            player_hurt_seconds: HURT_SECONDS,
            mob_life: Vitals::full(MOB_MAX_HP),
            mob_mode: MobMode::Patrol,
            mob_clock: 0.0,
            mob_spawn: mob,
            mob_hit_done: false,
            mob_attack_seconds: 0.9,
            mob_hit_seconds: 0.45,
            mob_hurt_seconds: HURT_SECONDS,
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
    props: Vec<PropBatch>,
    show_props: bool,
    player_model: Option<ActorModel>,
    mob_model: Option<ActorModel>,
    /// Head, helmet, weapon and shield attached to bones of the player model.
    attachments: Vec<Attachment>,
    player_yaw: f32,
    mob_yaw: f32,
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
    /// Points still to walk to, in sandbox coordinates (the click-to-move route).
    route: Vec<Vec2>,
    fight: Fight,
    mob_moving: bool,
    camera_yaw: f32,
    /// The original interface (windows and their images), handed to the renderer once.
    pending_ui: Option<(UiDesktop, Option<PakArchive>)>,
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
        let mob = walkable_at_distance(&map, player, 7.0).unwrap_or(player);
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
            props: Vec::new(),
            show_props: true,
            player_model: None,
            mob_model: None,
            attachments: Vec::new(),
            player_yaw: 0.0,
            mob_yaw: 0.0,
            baked_gain: 1.0,
            lights,
            player,
            mob,
            mob_direction: 0,
            input: MovementInput::default(),
            elapsed: 0.0,
            moving: false,
            route: Vec::new(),
            fight: Fight::new(player, mob),
            mob_moving: false,
            camera_yaw: 0.0,
            pending_ui: None,
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
            KeyCode::KeyO if pressed => self.show_props = !self.show_props,
            KeyCode::KeyB if pressed => {
                self.baked_gain = if self.baked_gain > 1.5 { 1.0 } else { 2.0 };
            }
            _ => {}
        }
    }

    fn update(&mut self, delta: f32, camera_yaw: f32) {
        self.elapsed += delta;
        self.camera_yaw = camera_yaw;
        let input = Vec2::new(
            f32::from(self.input.right) - f32::from(self.input.left),
            f32::from(self.input.forward) - f32::from(self.input.backward),
        );
        let free = self.fight.player_state == PlayerState::Free;
        let keyboard_direction = (input.length_squared() > 0.0 && free).then(|| {
            let input = input.normalize();
            let forward = Vec3::new(-camera_yaw.sin(), 0.0, -camera_yaw.cos());
            let right = Vec3::new(forward.z, 0.0, -forward.x);
            (right * input.x + forward * input.y).normalize_or_zero()
        });
        if keyboard_direction.is_some() {
            // Steering by hand cancels a click-to-move route and a pursuit.
            self.route.clear();
            self.fight.attack_target = false;
        }
        self.update_player(delta, keyboard_direction);
        self.update_mob_ai(delta);
        for batch in &mut self.props {
            let Some(animated) = &mut batch.animated else {
                continue;
            };
            animated.model.animate(delta, true);
            batch.vertices.clear();
            for object in &animated.instances {
                append_instance(
                    &mut batch.vertices,
                    &animated.model.vertices,
                    object,
                    &self.map,
                );
            }
        }
        if let Some(model) = &mut self.player_model {
            model.animate(delta, self.moving);
            if let Some(rig) = &model.rig {
                for part in &mut self.attachments {
                    part.follow(rig);
                }
            }
        }
        let mob_moving = self.mob_moving;
        if let Some(model) = &mut self.mob_model {
            model.animate(delta, mob_moving);
        }
    }

    /// Player state machine: free (walk, chase), swinging, reacting to a hit, or down.
    fn update_player(&mut self, delta: f32, keyboard_direction: Option<Vec3>) {
        self.moving = false;
        match self.fight.player_state {
            PlayerState::Dead => {
                self.fight.player_clock += delta;
                if self.fight.player_clock >= PLAYER_RESPAWN_SECONDS {
                    self.respawn_player();
                }
            }
            PlayerState::Hurt => {
                self.fight.player_clock += delta;
                if self.fight.player_clock >= self.fight.player_hurt_seconds {
                    self.fight.player_state = PlayerState::Free;
                }
            }
            PlayerState::Attacking => {
                self.fight.player_clock += delta;
                if !self.fight.player_hit_done
                    && self.fight.player_clock >= self.fight.player_hit_seconds
                {
                    self.fight.player_hit_done = true;
                    self.player_hits_mob();
                }
                if self.fight.player_clock >= self.fight.player_attack_seconds {
                    self.fight.player_state = PlayerState::Free;
                }
            }
            PlayerState::Free => {
                if let Some(direction) = keyboard_direction {
                    self.moving = true;
                    let speed = if self.input.run { 6.0 } else { 3.3 };
                    self.move_player(direction * speed * delta);
                } else if self.fight.attack_target {
                    self.chase_target(delta);
                } else {
                    self.moving = !self.route.is_empty();
                    self.follow_route(delta);
                }
            }
        }
    }

    /// Walks toward the clicked monster and swings once it is within reach.
    fn chase_target(&mut self, delta: f32) {
        if self.fight.mob_mode == MobMode::Dead {
            self.fight.attack_target = false;
            return;
        }
        let offset = Vec2::new(self.mob.x - self.player.x, self.mob.z - self.player.z);
        if offset.length() <= PLAYER_REACH {
            self.route.clear();
            self.player_yaw = offset.x.atan2(offset.y) + ACTOR_YAW_OFFSET;
            self.start_player_attack();
            return;
        }
        self.fight.replan -= delta;
        if self.route.is_empty() || self.fight.replan <= 0.0 {
            self.walk_to(self.mob, false);
            self.fight.replan = 0.4;
            if self.route.is_empty() {
                self.fight.attack_target = false;
                return;
            }
        }
        self.moving = !self.route.is_empty();
        self.follow_route(delta);
    }

    fn start_player_attack(&mut self) {
        self.fight.player_state = PlayerState::Attacking;
        self.fight.player_clock = 0.0;
        self.fight.player_hit_done = false;
        let slot = player_slot(self.fight.item_type, player_motion::ATTACK1_1);
        if let Some(model) = &mut self.player_model {
            model.play_action(slot, false);
        }
    }

    /// The blow lands (at the `.cdt` key frame of the swing): hurt the monster if it is in reach.
    fn player_hits_mob(&mut self) {
        let distance = Vec2::new(self.mob.x - self.player.x, self.mob.z - self.player.z).length();
        if self.fight.mob_mode == MobMode::Dead || distance > PLAYER_REACH * 1.4 {
            return;
        }
        let damage = player_damage_roll(self.fight.blows);
        self.fight.blows += 1;
        let killed = self.fight.mob_life.hurt(damage);
        eprintln!(
            "combat: you hit the monster for {damage} ({}/{})",
            self.fight.mob_life.hp, self.fight.mob_life.max
        );
        if killed {
            self.fight.mob_mode = MobMode::Dead;
            self.fight.mob_clock = 0.0;
            self.fight.attack_target = false;
            let slot = mob_slot(mob_motion::DOWN);
            if let Some(model) = &mut self.mob_model {
                model.play_action(slot, true);
            }
            eprintln!("combat: the monster is down");
        } else if self.fight.mob_mode != MobMode::Attack {
            // A swing in progress is not interrupted (like the player's), so it can hit back.
            self.fight.mob_mode = MobMode::Hurt;
            self.fight.mob_clock = 0.0;
            let slot = mob_slot(mob_motion::DEFENSEFAIL1);
            if let Some(model) = &mut self.mob_model {
                model.play_action(slot, false);
            }
        }
    }

    /// The monster's blow lands on the player.
    fn hurt_player(&mut self, amount: i32) {
        if self.fight.player_state == PlayerState::Dead {
            return;
        }
        let killed = self.fight.player_life.hurt(amount);
        eprintln!(
            "combat: the monster hits you for {amount} ({}/{})",
            self.fight.player_life.hp, self.fight.player_life.max
        );
        let item_type = self.fight.item_type;
        if killed {
            self.fight.player_state = PlayerState::Dead;
            self.fight.player_clock = 0.0;
            self.fight.attack_target = false;
            self.route.clear();
            if let Some(model) = &mut self.player_model {
                model.play_action(player_slot(item_type, player_motion::DYING), true);
            }
            eprintln!("combat: you are down; back on your feet in a moment");
        } else if self.fight.player_state == PlayerState::Free {
            // A swing in progress is not interrupted, so the fight cannot be stun-locked.
            self.fight.player_state = PlayerState::Hurt;
            self.fight.player_clock = 0.0;
            self.route.clear();
            if let Some(model) = &mut self.player_model {
                model.play_action(player_slot(item_type, player_motion::DEFENSEFAIL), false);
            }
        }
    }

    fn respawn_player(&mut self) {
        self.player = self.fight.player_start;
        self.fight.player_life = Vitals::full(PLAYER_MAX_HP);
        self.fight.player_state = PlayerState::Free;
        self.route.clear();
        if let Some(model) = &mut self.player_model {
            model.stop_action();
        }
        eprintln!("combat: you are back on your feet");
    }

    fn respawn_mob(&mut self) {
        self.mob = self.fight.mob_spawn;
        self.fight.mob_life = Vitals::full(MOB_MAX_HP);
        self.fight.mob_mode = MobMode::Patrol;
        if let Some(model) = &mut self.mob_model {
            model.stop_action();
        }
        eprintln!("combat: a new monster appears");
    }

    /// Monster brain: patrol, wake up and chase, swing, react to a blow, lie down and come back.
    fn update_mob_ai(&mut self, delta: f32) {
        let to_player = Vec2::new(self.player.x - self.mob.x, self.player.z - self.mob.z);
        let distance = to_player.length();
        let player_alive = self.fight.player_state != PlayerState::Dead;
        self.mob_moving = false;
        match self.fight.mob_mode {
            MobMode::Dead => {
                self.fight.mob_clock += delta;
                if self.fight.mob_clock >= MOB_RESPAWN_SECONDS {
                    self.respawn_mob();
                }
            }
            MobMode::Hurt => {
                self.fight.mob_clock += delta;
                if self.fight.mob_clock >= self.fight.mob_hurt_seconds {
                    self.fight.mob_mode = MobMode::Chase;
                }
            }
            mode => {
                let next = mob_decision(mode, distance, player_alive);
                if next != mode {
                    self.fight.mob_mode = next;
                    if next == MobMode::Attack {
                        self.start_mob_attack();
                    }
                }
                match self.fight.mob_mode {
                    MobMode::Patrol => {
                        self.mob_moving = true;
                        self.update_mob(delta);
                    }
                    MobMode::Chase => {
                        self.mob_moving = true;
                        self.chase_player(to_player, delta);
                    }
                    MobMode::Attack => {
                        if distance > 1e-3 {
                            self.mob_yaw = to_player.x.atan2(to_player.y) + ACTOR_YAW_OFFSET;
                        }
                        self.fight.mob_clock += delta;
                        if !self.fight.mob_hit_done
                            && self.fight.mob_clock >= self.fight.mob_hit_seconds
                        {
                            self.fight.mob_hit_done = true;
                            if distance <= MOB_REACH * 1.4 && player_alive {
                                self.hurt_player(MOB_DAMAGE);
                            }
                        }
                        if self.fight.mob_clock
                            >= self.fight.mob_attack_seconds + MOB_ATTACK_COOLDOWN
                        {
                            self.start_mob_attack();
                        }
                    }
                    MobMode::Hurt | MobMode::Dead => {}
                }
            }
        }
    }

    fn start_mob_attack(&mut self) {
        self.fight.mob_clock = 0.0;
        self.fight.mob_hit_done = false;
        let slot = mob_slot(mob_motion::ATTACK1);
        if let Some(model) = &mut self.mob_model {
            model.play_action(slot, false);
        }
    }

    /// Runs at the player, sliding along walls like the player does.
    fn chase_player(&mut self, to_player: Vec2, delta: f32) {
        let Some(direction) = to_player.try_normalize() else {
            return;
        };
        let step = direction * MOB_CHASE_SPEED * delta;
        let along_x = Vec3::new(self.mob.x + step.x, 0.0, self.mob.z);
        if self.can_stand(along_x, 0.24) {
            self.mob.x = along_x.x;
        }
        let along_z = Vec3::new(self.mob.x, 0.0, self.mob.z + step.y);
        if self.can_stand(along_z, 0.24) {
            self.mob.z = along_z.z;
        }
        self.mob_yaw = direction.x.atan2(direction.y) + ACTOR_YAW_OFFSET;
    }

    /// Clicking the monster (a ray through its body) orders an attack.
    fn mob_under_ray(&self, origin: Vec3, direction: Vec3) -> bool {
        self.fight.mob_mode != MobMode::Dead
            && ray_hits_sphere(
                origin.to_array(),
                direction.to_array(),
                [self.mob.x, MOB_BODY_HEIGHT * 0.5, self.mob.z],
                MOB_BODY_HEIGHT * 0.5,
            )
    }

    fn attack_mob(&mut self) {
        if self.fight.player_state == PlayerState::Dead || self.fight.mob_mode == MobMode::Dead {
            return;
        }
        self.route.clear();
        self.fight.attack_target = true;
        self.fight.replan = 0.0;
        eprintln!("combat: attacking the monster");
    }

    /// Wires the motions of the equipped weapon and the swing timings, from the `.chr` slots and the
    /// `.cdt` key frames; without them a swing lands halfway through.
    fn setup_combat(&mut self, item_type: u16, player_cdt: Option<&Cdt>, mob_cdt: Option<&Cdt>) {
        self.fight.item_type = item_type;
        if let Some(model) = &mut self.player_model {
            model.set_stance(
                player_slot(item_type, player_motion::STAND1),
                player_slot(item_type, player_motion::WALK),
            );
            let attack = player_slot(item_type, player_motion::ATTACK1_1);
            if let Some(length) = model.motion_duration(attack) {
                self.fight.player_attack_seconds = length;
                self.fight.player_hit_seconds = length * 0.5;
                let frame = player_cdt
                    .and_then(|cdt| {
                        cdt.effect_frames(u32::from(item_type), u32::from(player_motion::ATTACK1_1))
                    })
                    .map(|frames| u32::from(frames[0]))
                    .filter(|frame| *frame > 0);
                if let Some(seconds) = frame.and_then(|f| model.motion_seconds_at_frame(attack, f))
                {
                    self.fight.player_hit_seconds = seconds.min(length);
                }
            }
            if let Some(length) =
                model.motion_duration(player_slot(item_type, player_motion::DEFENSEFAIL))
            {
                self.fight.player_hurt_seconds = length;
            }
        }
        if let Some(model) = &mut self.mob_model {
            let attack = mob_slot(mob_motion::ATTACK1);
            if let Some(length) = model.motion_duration(attack) {
                self.fight.mob_attack_seconds = length;
                self.fight.mob_hit_seconds = length * 0.5;
                let frame = mob_cdt
                    .and_then(|cdt| cdt.effect_frames(0, u32::from(mob_motion::ATTACK1)))
                    .map(|frames| u32::from(frames[0]))
                    .filter(|frame| *frame > 0);
                if let Some(seconds) = frame.and_then(|f| model.motion_seconds_at_frame(attack, f))
                {
                    self.fight.mob_hit_seconds = seconds.min(length);
                }
            }
            if let Some(length) = model.motion_duration(mob_slot(mob_motion::DEFENSEFAIL1)) {
                self.fight.mob_hurt_seconds = length;
            }
        }
        eprintln!(
            "combat: weapon type {item_type}; swing {:.2}s (hit at {:.2}s), monster swing {:.2}s (hit at {:.2}s)",
            self.fight.player_attack_seconds,
            self.fight.player_hit_seconds,
            self.fight.mob_attack_seconds,
            self.fight.mob_hit_seconds
        );
    }

    /// Sandbox coordinates (tiles centred on the map) to map tile coordinates.
    fn to_tile_space(&self, point: Vec3) -> [f32; 2] {
        [
            point.x + self.map.width as f32 * 0.5,
            point.z + self.map.height as f32 * 0.5,
        ]
    }

    /// Plans a route to `target` over the walkable tiles and starts walking it.
    fn walk_to(&mut self, target: Vec3, announce: bool) {
        let path = navigation::find_path(
            &self.map,
            self.to_tile_space(self.player),
            self.to_tile_space(target),
            PLAYER_RADIUS,
        );
        match path {
            Some(points) => {
                if announce {
                    eprintln!(
                        "walk: {} waypoint(s) to tile ({:.1}, {:.1})",
                        points.len(),
                        points.last().map_or(0.0, |point| point[0]),
                        points.last().map_or(0.0, |point| point[1])
                    );
                }
                let half = Vec2::new(self.map.width as f32, self.map.height as f32) * 0.5;
                self.route = points
                    .iter()
                    .map(|point| Vec2::new(point[0], point[1]) - half)
                    .collect();
            }
            None => {
                if announce {
                    eprintln!("walk: no route to that spot");
                }
                self.route.clear();
            }
        }
    }

    /// Walks toward the next route point; stops if the way is blocked.
    fn follow_route(&mut self, delta: f32) {
        let Some(next) = self.route.first().copied() else {
            return;
        };
        let here = Vec2::new(self.player.x, self.player.z);
        let offset = next - here;
        let step = if self.input.run { 6.0 } else { 3.3 } * delta;
        if offset.length() <= step {
            self.player.x = next.x;
            self.player.z = next.y;
            self.route.remove(0);
            return;
        }
        let direction = offset.normalize();
        let before = self.player;
        self.move_player(Vec3::new(direction.x, 0.0, direction.y) * step);
        if (self.player - before).length() < step * 0.25 {
            // Blocked (a moving obstacle or rounding at a corner): give up instead of pushing.
            self.route.clear();
        }
    }

    fn move_player(&mut self, movement: Vec3) {
        if movement.length_squared() > 0.0 {
            self.player_yaw = movement.x.atan2(movement.z) + ACTOR_YAW_OFFSET;
        }
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
                self.mob_yaw = DIRECTIONS[self.mob_direction]
                    .x
                    .atan2(DIRECTIONS[self.mob_direction].z)
                    + ACTOR_YAW_OFFSET;
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
            if self.player_model.is_none() {
                add_humanoid(&mut vertices, self.player + Vec3::Y * bob);
            }
            let right = Vec3::new(self.camera_yaw.cos(), 0.0, -self.camera_yaw.sin());
            if self.fight.player_state != PlayerState::Dead {
                add_life_bar(
                    &mut vertices,
                    self.player + Vec3::Y * PLAYER_BAR_HEIGHT,
                    self.fight.player_life.fraction(),
                    right,
                    [0.25, 0.85, 0.3],
                );
            }
            if self.fight.mob_mode != MobMode::Dead {
                add_life_bar(
                    &mut vertices,
                    self.mob + Vec3::Y * MOB_BAR_HEIGHT,
                    self.fight.mob_life.fraction(),
                    right,
                    [0.9, 0.15, 0.15],
                );
            }
            if let Some(destination) = self.route.last() {
                add_pyramid(
                    &mut vertices,
                    Vec3::new(destination.x, 0.0, destination.y),
                    0.12,
                    0.35,
                    [1.0, 0.85, 0.2],
                );
            }
            if self.mob_model.is_none() {
                add_mob(
                    &mut vertices,
                    self.mob + Vec3::Y * ((self.elapsed * 4.0).sin() * 0.06),
                );
            }
        }
        vertices
    }

    /// The actor's triangles placed in the world: rotated to its heading, moved to its position.
    fn actor_vertices(&self, kind: ActorKind) -> Vec<Vertex> {
        if !self.show_entities {
            return Vec::new();
        }
        let (model, position, yaw) = match kind {
            ActorKind::Player => (self.player_model.as_ref(), self.player, self.player_yaw),
            ActorKind::Mob => (self.mob_model.as_ref(), self.mob, self.mob_yaw),
            ActorKind::Attached(index) => (
                self.attachments.get(index).map(|part| &part.model),
                self.player,
                self.player_yaw,
            ),
        };
        let Some(model) = model else {
            return Vec::new();
        };
        // Head, helmet, weapon and shield turn with the body they are attached to.
        let facing = match kind {
            ActorKind::Mob => model.facing,
            ActorKind::Player | ActorKind::Attached(_) => {
                self.player_model.as_ref().map_or(0.0, |body| body.facing)
            }
        };
        let rotation = glam::Quat::from_rotation_y(yaw + facing);
        model
            .vertices
            .iter()
            .map(|vertex| {
                let mut placed = *vertex;
                placed.position =
                    (rotation * Vec3::from_array(vertex.position) + position).to_array();
                placed.normal = (rotation * Vec3::from_array(vertex.normal)).to_array();
                placed
            })
            .collect()
    }

    fn maximum_vertex_count(&self) -> usize {
        self.stm_vertices.len() + self.map_vertices.len() + 200 + 128
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

/// A walkable tile about `distance` tiles from `from`: the monster starts near enough to find.
fn walkable_at_distance(map: &TileMap, from: Vec3, distance: f32) -> Option<Vec3> {
    (0..map.tiles.len())
        .filter(|index| map.tiles[*index].is_walkable())
        .map(|index| tile_center(map, index))
        .min_by(|a, b| {
            let error = |point: &Vec3| ((*point - from).length() - distance).abs();
            error(a).total_cmp(&error(b))
        })
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
            let stm_blend = textures.blend_class(layer, false);
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
                    let vertex =
                        Vertex::textured(position, normal, color, uv, layer).with_blend(stm_blend);
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

/// A flat life bar that faces the camera: dark back plate and a fill of `fraction` (0..1) of the width.
fn add_life_bar(
    vertices: &mut Vec<Vertex>,
    anchor: Vec3,
    fraction: f32,
    right: Vec3,
    fill: [f32; 3],
) {
    const WIDTH: f32 = 1.0;
    const HEIGHT: f32 = 0.10;
    let normal = right.cross(Vec3::Y);
    let start = anchor - right * (WIDTH * 0.5);
    let up = Vec3::Y * HEIGHT;
    add_quad(
        vertices,
        [
            start,
            start + right * WIDTH,
            start + right * WIDTH + up,
            start + up,
        ],
        normal,
        [0.08, 0.08, 0.08],
    );
    if fraction > 0.0 {
        // The fill sits a hair in front of the plate and a little inside its border.
        let lift = normal * 0.004;
        let inset = right * 0.012 + Vec3::Y * 0.012;
        let end = start + right * (WIDTH * fraction);
        add_quad(
            vertices,
            [
                start + inset + lift,
                end - right * 0.012 + Vec3::Y * 0.012 + lift,
                end - right * 0.012 + up - Vec3::Y * 0.012 + lift,
                start + inset + up - Vec3::Y * 0.024 + lift,
            ],
            normal,
            fill,
        );
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
    /// One of the `BLEND_*` classes.
    blend: f32,
}

impl Vertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 9] = wgpu::vertex_attr_array![
        0 => Float32x3,
        1 => Float32x3,
        2 => Float32x3,
        3 => Float32x2,
        4 => Float32,
        5 => Float32,
        6 => Float32x2,
        7 => Float32,
        8 => Float32,
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
            blend: BLEND_OPAQUE,
        }
    }

    fn with_blend(mut self, blend: f32) -> Self {
        self.blend = blend;
        self
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
    /// Pixels the cursor has travelled since the button went down.
    drag_distance: f32,
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
            drag_distance: 0.0,
            last_cursor: None,
        }
    }

    fn cursor_moved(&mut self, position: PhysicalPosition<f64>) {
        if self.dragging
            && let Some(previous) = self.last_cursor
        {
            self.drag_distance +=
                ((position.x - previous.x).abs() + (position.y - previous.y).abs()) as f32;
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

/// The world ray (origin, unit direction) through `ndc` (normalised device coordinates, y up).
/// The projection has Direct3D-style depth: 0 at the near plane, 1 at the far plane.
fn unproject_ray(view_projection: [[f32; 4]; 4], ndc: Vec2) -> (Vec3, Vec3) {
    let inverse = Mat4::from_cols_array_2d(&view_projection).inverse();
    let near = inverse.project_point3(ndc.extend(0.0));
    let far = inverse.project_point3(ndc.extend(1.0));
    (near, (far - near).normalize_or_zero())
}

/// Where the ray through `ndc` meets the plane `y = ground`.
fn unproject_to_ground(view_projection: [[f32; 4]; 4], ndc: Vec2, ground: f32) -> Option<Vec3> {
    let (origin, direction) = unproject_ray(view_projection, ndc);
    if direction.y.abs() < 1e-6 {
        return None;
    }
    let along = (ground - origin.y) / direction.y;
    (along > 0.0).then(|| origin + direction * along)
}

struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    pipeline: wgpu::RenderPipeline,
    blend_pipeline: wgpu::RenderPipeline,
    additive_pipeline: wgpu::RenderPipeline,
    /// Whether anything is alpha-blended / additive, so empty passes are skipped.
    uses_blend: [bool; 2],
    texture_bind_group: wgpu::BindGroup,
    actors: Vec<ActorGpu>,
    props: Vec<PropGpu>,
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
    ui: Option<UiGpu>,
    /// The last press went to an interface window, so the release is not a move order.
    ui_captured: bool,
    game_mode: bool,
}

impl Renderer {
    async fn new(
        window: Arc<Window>,
        scene: SandboxScene,
        textures: MapTextures,
    ) -> Result<Self, String> {
        let mut scene = scene;
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
        // Three pipelines share the buffers and keep only the vertices of their own blend class.
        // The blended ones read the depth buffer but do not write it.
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let make_pipeline = |label: &str, fragment: &str, blend: wgpu::BlendState, depth_write| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
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
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: config.format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make_pipeline(
            "sandbox opaque pipeline",
            "fragment_opaque",
            wgpu::BlendState::REPLACE,
            true,
        );
        let blend_pipeline = make_pipeline(
            "sandbox alpha pipeline",
            "fragment_alpha",
            wgpu::BlendState::ALPHA_BLENDING,
            false,
        );
        let additive_pipeline = make_pipeline(
            "sandbox additive pipeline",
            "fragment_additive",
            additive,
            false,
        );
        let vertices = scene.vertices();
        let has_class = |class: f32| {
            let is_class = |vertex: &Vertex| vertex.blend == class;
            vertices.iter().any(is_class)
                || scene
                    .props
                    .iter()
                    .any(|batch| batch.vertices.iter().any(is_class))
                || [scene.player_model.as_ref(), scene.mob_model.as_ref()]
                    .into_iter()
                    .flatten()
                    .chain(scene.attachments.iter().map(|part| &part.model))
                    .any(|model| model.vertices.iter().any(is_class))
        };
        let uses_blend = [has_class(BLEND_ALPHA), has_class(BLEND_ADDITIVE)];
        let vertex_capacity = scene.maximum_vertex_count();
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sandbox vertices"),
            size: (vertex_capacity * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        let depth = DepthTarget::new(&device, width, height);
        let props: Vec<PropGpu> = scene
            .props
            .iter()
            .map(|batch| {
                let (_layout, bind_group) = batch.textures.upload(&device, &queue);
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("sandbox prop vertices"),
                    contents: bytemuck::cast_slice(&batch.vertices),
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                });
                PropGpu {
                    bind_group,
                    buffer,
                    count: batch.vertices.len() as u32,
                }
            })
            .collect();
        let mut actors = Vec::new();
        let attached = scene
            .attachments
            .iter()
            .enumerate()
            .map(|(index, part)| (ActorKind::Attached(index), Some(&part.model)));
        let placed_actors = [
            (ActorKind::Player, scene.player_model.as_ref()),
            (ActorKind::Mob, scene.mob_model.as_ref()),
        ]
        .into_iter()
        .chain(attached);
        for (kind, model) in placed_actors {
            let Some(model) = model else {
                continue;
            };
            let (_layout, bind_group) = model.textures.upload(&device, &queue);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("sandbox actor vertices"),
                size: (model.vertices.len() * std::mem::size_of::<Vertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            actors.push(ActorGpu {
                kind,
                bind_group,
                buffer,
                count: 0,
            });
        }
        let ui = scene.pending_ui.take().map(|(desktop, archive)| {
            UiGpu::new(&device, config.format, DEPTH_FORMAT, archive, desktop)
        });
        eprintln!("uploaded {} triangles", vertices.len() / 3);
        Ok(Self {
            surface,
            device,
            queue,
            config,
            size,
            pipeline,
            blend_pipeline,
            additive_pipeline,
            uses_blend,
            texture_bind_group,
            actors,
            props,
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
            ui,
            ui_captured: false,
            game_mode: game_mode(),
        })
    }

    /// The cursor as normalised device coordinates (y up), or `None` before the window has a size.
    fn cursor_ndc(&self) -> Option<Vec2> {
        let cursor = self.camera.last_cursor?;
        let (width, height) = (self.config.width as f32, self.config.height as f32);
        (width >= 1.0 && height >= 1.0).then(|| {
            Vec2::new(
                2.0 * cursor.x as f32 / width - 1.0,
                1.0 - 2.0 * cursor.y as f32 / height,
            )
        })
    }

    fn view_projection(&self) -> [[f32; 4]; 4] {
        let (width, height) = (self.config.width as f32, self.config.height as f32);
        self.camera.uniform(width / height.max(1.0)).view_projection
    }

    /// A click orders an attack on the monster under the cursor, or else a walk to the ground there.
    fn click_to_move(&mut self) {
        let Some(ndc) = self.cursor_ndc() else {
            return;
        };
        let view_projection = self.view_projection();
        let (origin, direction) = unproject_ray(view_projection, ndc);
        if self.scene.mob_under_ray(origin, direction) {
            self.scene.attack_mob();
            return;
        }
        self.scene.fight.attack_target = false;
        if let Some(target) = unproject_to_ground(view_projection, ndc, self.scene.player.y) {
            self.scene.walk_to(target, true);
        }
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
        for (prop, batch) in self.props.iter().zip(&self.scene.props) {
            if batch.animated.is_some() {
                self.queue
                    .write_buffer(&prop.buffer, 0, bytemuck::cast_slice(&batch.vertices));
            }
        }
        if let Some(ui) = &mut self.ui {
            ui.prepare(
                &self.device,
                &self.queue,
                (self.config.width, self.config.height),
            );
        }
        for actor in &mut self.actors {
            let placed = self.scene.actor_vertices(actor.kind);
            actor.count = placed.len() as u32;
            self.queue
                .write_buffer(&actor.buffer, 0, bytemuck::cast_slice(&placed));
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
            pass.set_bind_group(0, &self.camera_bind_group, &[]);
            // Opaque first, then alpha-blended, then additive, so translucent surfaces and glows
            // sit on top of what is behind them.
            let passes = [
                (&self.pipeline, true),
                (&self.blend_pipeline, self.uses_blend[0]),
                (&self.additive_pipeline, self.uses_blend[1]),
            ];
            for (pipeline, enabled) in passes {
                if !enabled {
                    continue;
                }
                pass.set_pipeline(pipeline);
                pass.set_bind_group(1, &self.texture_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.draw(0..self.vertex_count, 0..1);
                if self.scene.show_props {
                    for prop in &self.props {
                        pass.set_bind_group(1, &prop.bind_group, &[]);
                        pass.set_vertex_buffer(0, prop.buffer.slice(..));
                        pass.draw(0..prop.count, 0..1);
                    }
                }
                for actor in self.actors.iter().filter(|actor| actor.count > 0) {
                    pass.set_bind_group(1, &actor.bind_group, &[]);
                    pass.set_vertex_buffer(0, actor.buffer.slice(..));
                    pass.draw(0..actor.count, 0..1);
                }
            }
            // The interface goes on top of everything, without depth.
            if let Some(ui) = &self.ui {
                ui.draw(&mut pass);
            }
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActorKind {
    Player,
    Mob,
    /// A part attached to a bone of the player's skeleton (head, helmet, weapon, shield).
    Attached(usize),
}

/// A textured `.MOD` in sandbox units with the origin at its feet. Its vertices start in the
/// bind pose (the raw vertex positions already are the bind pose); actors loaded with a rig are
/// re-posed every frame from a `.ANM`.
struct ActorModel {
    /// Turn (radians) that makes the model's own front line up with the direction it moves in.
    facing: f32,
    vertices: Vec<Vertex>,
    textures: MapTextures,
    rig: Option<Rig>,
}

/// Posed positions and normals of one mesh, in sandbox units.
type PosedMesh = (Vec<[f32; 3]>, Vec<[f32; 3]>);

/// One mesh of a rigged model, in model units.
struct RigMesh {
    /// Node index of the mesh itself, used when the mesh has no skin (it then follows its node).
    node: usize,
    rest_positions: Vec<Vec3>,
    rest_normals: Vec<Vec3>,
    /// Per vertex: `(node index, weight)` of each bone that moves it. `None` for rigid meshes.
    influences: Option<Vec<Vec<(usize, f32)>>>,
}

/// Everything needed to animate an actor: the skeleton, the meshes with their skin, which
/// triangle corner comes from which mesh vertex, and the motions with their playback state.
struct Rig {
    skeleton: Skeleton,
    meshes: Vec<RigMesh>,
    /// For each entry of `ActorModel::vertices`: `(mesh, vertex)`.
    corners: Vec<(u32, u32)>,
    /// Model units to sandbox units.
    scale: f32,
    /// World matrix of every node in the pose last shown (the bind pose until a motion plays).
    pose: Vec<Matrix>,
    /// One entry per `.chr` slot; `None` for a missing or blank (`blank_player_ani`) motion.
    motions: Vec<Option<MotionFile>>,
    idle: Option<usize>,
    moving: Option<usize>,
    current: Option<usize>,
    clock: f32,
    /// A motion played once (an attack, a hit, falling down) that overrides idle and walking.
    action: Option<Action>,
    action_restart: bool,
}

struct Action {
    slot: usize,
    /// Stay on the last frame when it ends (lying down) instead of going back to idle.
    hold: bool,
}

/// Motion slots to play while standing and while walking, from the client's motion tables:
/// a monster `.chr` has one slot per `MON_MOTION_TYPE` (`STAND1 = 1` is slot 0, `MOVE1 = 3` is
/// slot 2); a player character uses `WALK = 7` (slot 6); an NPC has a single motion.
fn default_motion_slots(package: &str) -> (Option<usize>, Option<usize>) {
    match package.to_ascii_lowercase().as_str() {
        "monster" => (Some(0), Some(2)),
        "character" => (Some(0), Some(6)),
        _ => (Some(0), None),
    }
}

impl ActorModel {
    fn load(
        ttb_path: &Path,
        package: &str,
        entry: &str,
        tile_size: f32,
        anim_variable: &str,
    ) -> Result<Self, String> {
        let archive = open_data_package(ttb_path, package)?;
        let bytes = archive
            .read_entry(entry)
            .map_err(|error| error.to_string())?;
        // The textures of a model live in its own package (`.dds`, sometimes `.tif`).
        let packages = TexturePackages {
            dds: vec![archive.clone()],
            tif: vec![archive],
        };
        let mut model = Self::from_bytes(&bytes, &packages, tile_size, true)?;
        model.facing = facing_offset(package);
        let (idle, moving) = motion_slots_override(package, anim_variable);
        model.attach_motions(&packages.dds[0], entry, idle, moving);
        eprintln!(
            "loaded actor {package}/{entry}: {} triangles{}",
            model.vertices.len() / 3,
            model
                .rig
                .as_ref()
                .map(|rig| format!(
                    ", {} motions (idle {:?}, moving {:?})",
                    rig.motions.iter().flatten().count(),
                    rig.idle,
                    rig.moving
                ))
                .unwrap_or_default()
        );
        Ok(model)
    }

    /// Loads the model's `.chr` manifest and the motions it lists from the same package.
    fn attach_motions(
        &mut self,
        archive: &PakArchive,
        entry: &str,
        idle: Option<usize>,
        moving: Option<usize>,
    ) {
        let Some(rig) = &mut self.rig else {
            return;
        };
        let stem = entry.rsplit_once('.').map_or(entry, |(stem, _)| stem);
        let names = archive
            .read_entry(&format!("{stem}.chr"))
            .ok()
            .and_then(|bytes| ChrManifest::parse(&bytes).ok())
            .map_or_else(|| vec![format!("{stem}.anm")], |manifest| manifest.motions);
        rig.motions = names
            .iter()
            .map(|name| {
                let bytes = archive.read_entry(name).ok()?;
                MotionFile::parse(&bytes)
                    .ok()
                    .filter(|motion| motion.records.iter().any(|record| record.has_tracks()))
            })
            .collect();
        let available = |slot: Option<usize>| {
            slot.filter(|slot| rig.motions.get(*slot).is_some_and(Option::is_some))
        };
        rig.idle = available(idle);
        rig.moving = available(moving);
    }

    fn motion_at(&self, slot: usize) -> Option<&MotionFile> {
        self.rig.as_ref()?.motions.get(slot)?.as_ref()
    }

    /// Length in seconds of the motion in `slot`, if it exists.
    fn motion_duration(&self, slot: usize) -> Option<f32> {
        self.motion_at(slot).map(MotionFile::duration_seconds)
    }

    /// Time at which the motion in `slot` reaches a `.cdt` frame number.
    fn motion_seconds_at_frame(&self, slot: usize, frame: u32) -> Option<f32> {
        self.motion_at(slot)
            .map(|motion| motion.seconds_at_frame(frame))
    }

    /// Plays the motion in `slot` once from its start; `false` if the model has no such motion.
    fn play_action(&mut self, slot: usize, hold: bool) -> bool {
        if self.motion_at(slot).is_none() {
            return false;
        }
        if let Some(rig) = &mut self.rig {
            rig.action = Some(Action { slot, hold });
            rig.action_restart = true;
        }
        true
    }

    fn stop_action(&mut self) {
        if let Some(rig) = &mut self.rig {
            rig.action = None;
        }
    }

    /// Standing and walking motions, when the model has them (the weapon in hand changes the set).
    fn set_stance(&mut self, idle: usize, moving: usize) {
        let has = |model: &Self, slot: usize| model.motion_at(slot).is_some();
        let (idle_ok, moving_ok) = (has(self, idle), has(self, moving));
        if let Some(rig) = &mut self.rig {
            if idle_ok {
                rig.idle = Some(idle);
            }
            if moving_ok {
                rig.moving = Some(moving);
            }
        }
    }

    /// Advances the playing motion and re-poses every vertex.
    fn animate(&mut self, delta: f32, moving: bool) {
        let Self { vertices, rig, .. } = self;
        let Some(rig) = rig else {
            return;
        };
        let wanted = if let Some(action) = &rig.action {
            Some(action.slot)
        } else if moving {
            rig.moving.or(rig.idle)
        } else {
            rig.idle
        };
        if wanted != rig.current || rig.action_restart {
            rig.current = wanted;
            rig.clock = 0.0;
            rig.action_restart = false;
        }
        let Some(Some(motion)) = rig.current.and_then(|slot| rig.motions.get(slot)) else {
            return;
        };
        rig.clock += delta;
        let once = rig.action.is_some();
        let frame = if once {
            motion.frame_clamped_at(rig.clock)
        } else {
            motion.frame_at(rig.clock)
        };
        // A motion played once that has run its course hands the actor back to idle or walking.
        if rig
            .action
            .as_ref()
            .is_some_and(|action| !action.hold && rig.clock >= motion.duration_seconds())
        {
            rig.action = None;
        }
        let tracks = rig.skeleton.tracks_for(motion);
        let pose = rig.skeleton.pose(&tracks, frame);
        rig.pose.clone_from(&pose);
        // `inverse bind * posed world` moves a point from the bind pose to the posed one.
        let moves: Vec<_> = (0..pose.len())
            .map(|node| rig.skeleton.rigid_matrix(node, &pose))
            .collect();

        let posed: Vec<PosedMesh> = rig
            .meshes
            .iter()
            .map(|mesh| {
                let mut positions = Vec::with_capacity(mesh.rest_positions.len());
                let mut normals = Vec::with_capacity(mesh.rest_positions.len());
                for (index, (rest, rest_normal)) in mesh
                    .rest_positions
                    .iter()
                    .zip(&mesh.rest_normals)
                    .enumerate()
                {
                    let (position, normal) = match mesh.influences.as_ref().map(|all| &all[index]) {
                        Some(list) if !list.is_empty() => {
                            let (mut position, mut normal) = ([0.0_f32; 3], [0.0_f32; 3]);
                            for (node, weight) in list {
                                let p = transform_point(&moves[*node], rest.to_array());
                                let n = transform_vector(&moves[*node], rest_normal.to_array());
                                for axis in 0..3 {
                                    position[axis] += weight * p[axis];
                                    normal[axis] += weight * n[axis];
                                }
                            }
                            (position, normal)
                        }
                        _ => (
                            transform_point(&moves[mesh.node], rest.to_array()),
                            transform_vector(&moves[mesh.node], rest_normal.to_array()),
                        ),
                    };
                    positions.push((Vec3::from_array(position) * rig.scale).to_array());
                    normals.push(Vec3::from_array(normal).normalize_or(Vec3::Y).to_array());
                }
                (positions, normals)
            })
            .collect();
        for (vertex, (mesh, index)) in vertices.iter_mut().zip(&rig.corners) {
            let (positions, normals) = &posed[*mesh as usize];
            vertex.position = positions[*index as usize];
            vertex.normal = normals[*index as usize];
        }
    }

    /// Builds the model from `.MOD` bytes, resolving its textures from `packages`.
    fn from_bytes(
        bytes: &[u8],
        packages: &TexturePackages,
        tile_size: f32,
        with_rig: bool,
    ) -> Result<Self, String> {
        let model = ModelFile::parse(bytes).map_err(|error| error.to_string())?;
        let names: Vec<String> = model
            .materials
            .iter()
            .map(|material| material.texture_name.clone())
            .collect();
        let textures = MapTextures::from_names(&names, packages);

        let scale = 1.0 / tile_size;
        let skeleton = with_rig.then(|| Skeleton::new(&model));
        let mut vertices = Vec::new();
        let mut corner_sources: Vec<(u32, u32)> = Vec::new();
        let mut rig_meshes: Vec<RigMesh> = Vec::new();
        for mesh in &model.meshes {
            let Some(geometry) = &mesh.geometry else {
                continue;
            };
            let mesh_slot = rig_meshes.len() as u32;
            let points: Vec<Vec3> = geometry
                .positions
                .iter()
                .map(|position| Vec3::from_array(*position) * scale)
                .collect();
            // Smooth normals: area-weighted average of the faces around each vertex.
            let mut normals = vec![Vec3::ZERO; points.len()];
            for face in geometry.face_groups.iter().flat_map(|group| &group.faces) {
                let [a, b, c] = face.map(usize::from);
                if a.max(b).max(c) >= points.len() {
                    continue;
                }
                let normal = (points[b] - points[a]).cross(points[c] - points[a]);
                normals[a] += normal;
                normals[b] += normal;
                normals[c] += normal;
            }
            if let Some(skeleton) = &skeleton {
                let influences = geometry.skin.as_ref().map(|skin| {
                    skin.influences
                        .iter()
                        .map(|list| {
                            let total: f32 = list.iter().map(|item| item.weight).sum();
                            list.iter()
                                .filter_map(|item| {
                                    let node = skeleton.index_of_id(item.bone_id)?;
                                    (total > 1e-6).then_some((node, item.weight / total))
                                })
                                .collect()
                        })
                        .collect()
                });
                rig_meshes.push(RigMesh {
                    node: skeleton.index_of_id(mesh.id).unwrap_or(0),
                    rest_positions: geometry
                        .positions
                        .iter()
                        .map(|position| Vec3::from_array(*position))
                        .collect(),
                    rest_normals: normals
                        .iter()
                        .map(|normal| normal.normalize_or(Vec3::Y))
                        .collect(),
                    influences,
                });
            }
            for group in &geometry.face_groups {
                // The group's `material` is a selector, not an index (see the assets README).
                let material = model.material_index_for_group(group.material_index);
                let layer = material.and_then(|index| textures.layer_for_material(index as u32));
                let additive = material
                    .is_some_and(|index| model.materials[index].flags & MATERIAL_ADDITIVE != 0);
                let blend = textures.blend_class(layer, additive);
                let color = match (layer, material) {
                    (Some(_), _) => [1.0; 3],
                    (None, Some(index)) => {
                        material_color(&model.materials[index].texture_name, group.material_index)
                    }
                    (None, None) => material_color("missing", group.material_index),
                };
                let layer = layer.map_or(-1.0, |layer| layer as f32);
                for face in &group.faces {
                    let corners = face.map(usize::from);
                    if corners.iter().any(|index| *index >= points.len()) {
                        continue;
                    }
                    for corner in corners {
                        corner_sources.push((mesh_slot, corner as u32));
                        let uv = geometry
                            .texture_coordinates
                            .get(corner)
                            .copied()
                            .unwrap_or([0.0, 0.0]);
                        vertices.push(
                            Vertex::textured(
                                points[corner],
                                normals[corner].normalize_or(Vec3::Y),
                                color,
                                uv,
                                layer,
                            )
                            .with_blend(blend),
                        );
                    }
                }
            }
        }
        if vertices.is_empty() {
            return Err("the model has no decodable mesh".to_owned());
        }
        let rig = skeleton.map(|skeleton| Rig {
            pose: skeleton.bind_world().to_vec(),
            skeleton,
            meshes: rig_meshes,
            corners: corner_sources,
            scale,
            motions: Vec::new(),
            idle: None,
            moving: None,
            current: None,
            clock: 0.0,
            action: None,
            action_restart: false,
        });
        Ok(Self {
            facing: 0.0,
            vertices,
            textures,
            rig,
        })
    }
}

/// A model attached to a bone of the player's skeleton, like `ItemAttach` and the head attach of the
/// original client (`CodeFun.cpp`, `DungeonProcess.cpp`): the model's pivot sits on the bone and
/// moves with it.
struct Attachment {
    model: ActorModel,
    /// The model's vertices in its own space, before the bone matrix is applied.
    local: Vec<Vertex>,
    bone: usize,
}

impl Attachment {
    /// Loads `entry` from the first of `packages` that has it and attaches it to `bone_name`.
    #[allow(clippy::too_many_arguments)]
    fn load(
        ttb_path: &Path,
        rig: &Rig,
        bone_name: &str,
        label: &str,
        packages: &[&str],
        entry: &str,
        tile_size: f32,
    ) -> Option<Self> {
        let Some(bone) = rig.skeleton.index_of_name(bone_name) else {
            eprintln!("{label}: the player model has no bone `{bone_name}`");
            return None;
        };
        for package in packages {
            let Ok(archive) = open_data_package(ttb_path, package) else {
                continue;
            };
            let Ok(mut bytes) = archive.read_entry(entry) else {
                continue;
            };
            // Animated items (`.chr`) are a manifest that names the `.mod`; the sandbox shows
            // the bind pose of the model and ignores the item's own motions.
            if entry.ends_with(".chr") {
                let Ok(manifest) = ChrManifest::parse(&bytes) else {
                    continue;
                };
                let Ok(model_bytes) = archive.read_entry(&manifest.model_file) else {
                    eprintln!("{label}: {} is not in {package}", manifest.model_file);
                    continue;
                };
                bytes = model_bytes;
            }
            let textures = TexturePackages {
                dds: vec![archive.clone()],
                tif: vec![archive],
            };
            match ActorModel::from_bytes(&bytes, &textures, tile_size, false) {
                Ok(model) => {
                    // The model is authored away from the origin: the engine fixes the pivot of
                    // its node (the node's world translation) to the bone, so the vertices are
                    // moved into the node's own space first.
                    let inverse = ModelFile::parse(&bytes)
                        .ok()
                        .and_then(|parsed| parsed.nodes.first().map(|node| node.inverse));
                    let units = 1.0 / rig.scale;
                    let local: Vec<Vertex> = model
                        .vertices
                        .iter()
                        .map(|vertex| {
                            let mut moved = *vertex;
                            if let Some(inverse) = &inverse {
                                let model_point = Vec3::from_array(vertex.position) * units;
                                moved.position = (Vec3::from_array(transform_point(
                                    inverse,
                                    model_point.to_array(),
                                )) * rig.scale)
                                    .to_array();
                                moved.normal = transform_vector(inverse, vertex.normal);
                            }
                            moved
                        })
                        .collect();
                    eprintln!(
                        "{label}: {package}/{entry} on `{bone_name}`, {} triangles",
                        model.vertices.len() / 3
                    );
                    return Some(Self { model, local, bone });
                }
                Err(error) => eprintln!("{label}: {entry}: {error}"),
            }
        }
        eprintln!(
            "{label}: {entry} is not in the {} package(s)",
            packages.join("/")
        );
        None
    }

    /// Moves the model with its bone in the player's current pose.
    fn follow(&mut self, rig: &Rig) {
        let Some(mut matrix) = rig.pose.get(self.bone).copied() else {
            return;
        };
        // The bone matrix is in model units; the model's vertices already are in sandbox units.
        for value in &mut matrix[3][..3] {
            *value *= rig.scale;
        }
        for (placed, local) in self.model.vertices.iter_mut().zip(&self.local) {
            placed.position = transform_point(&matrix, local.position);
            placed.normal = transform_vector(&matrix, local.normal);
        }
    }
}

/// Base body of a class when no armor is worn (`RESTYPE_BASE_BODY`, `pm0<class>000`).
fn base_body_entry(class: u16) -> String {
    format!("pm{class:02}000.mod")
}

/// Head model: `RESTYPE_HEAD_MALE` for classes below 4, `_FEMALE` otherwise; the head id `1001`
/// names `ph101001.mod` (and `ph201001.mod` for the female ones).
fn head_entry(class: u16, head: u16) -> String {
    let sex = if class < 4 { 1 } else { 2 };
    format!("ph{sex}0{head}.mod")
}

/// A number from an environment variable.
fn env_number(name: &str) -> Option<u16> {
    env::var(name).ok()?.trim().parse().ok()
}

/// What the character wears, as the appearance packet of the original client describes it
/// (`DungeonProcess.cpp`): `wArmor` is the whole **body** model, `wHead` the head model, `wHelmet`
/// sits on the head bone, `wHandR` and `wHandL` on the hands. Read from `CORUM_CLASS` (1 warrior,
/// 2 priest, 3 summoner, 4 hunter, 5 wizard), `CORUM_ARMOR`, `CORUM_HEAD`, `CORUM_HELMET`,
/// `CORUM_ITEM` (right hand) and `CORUM_SHIELD` (left hand); `None` when none is set.
struct Outfit {
    class: u16,
    armor: u16,
    head: u16,
    helmet: u16,
    right: u16,
    left: u16,
}

impl Outfit {
    fn from_env() -> Option<Self> {
        let names = [
            "CORUM_CLASS",
            "CORUM_ARMOR",
            "CORUM_HEAD",
            "CORUM_HELMET",
            "CORUM_ITEM",
            "CORUM_SHIELD",
        ];
        names
            .iter()
            .any(|name| env::var_os(name).is_some())
            .then(|| Self {
                class: env_number("CORUM_CLASS").unwrap_or(1).clamp(1, 5),
                armor: env_number("CORUM_ARMOR").unwrap_or(0),
                head: env_number("CORUM_HEAD").unwrap_or(0),
                helmet: env_number("CORUM_HELMET").unwrap_or(0),
                right: env_number("CORUM_ITEM").unwrap_or(0),
                left: env_number("CORUM_SHIELD").unwrap_or(0),
            })
    }

    /// Builds the body and the parts attached to it.
    fn build(&self, ttb_path: &Path, tile_size: f32) -> (Option<ActorModel>, Vec<Attachment>) {
        let catalog = data_directories(ttb_path)
            .into_iter()
            .find(|directory| directory.join("Manager").join("ItemResource.cdb").is_file())
            .and_then(|directory| {
                ItemCatalog::load(&directory.join("Manager"))
                    .map_err(|error| eprintln!("outfit: {error}"))
                    .ok()
            });
        let name_of = |id: u16| {
            catalog
                .as_ref()
                .and_then(|catalog| catalog.items.get(&id))
                .map_or_else(String::new, |item| item.name_eng.lossy())
        };
        // No armor: the base body of the class (`RESTYPE_BASE_BODY`, `pm0<class>000`).
        // Armor: `ItemDataName(armor, class - 1)`, that is `<model>_<class - 1, 3 digits>.mod`.
        let body_entry = if self.armor == 0 {
            Some(base_body_entry(self.class))
        } else {
            catalog
                .as_ref()
                .and_then(|catalog| catalog.model_entry(self.armor, self.class - 1))
        };
        // A body item is a `.chr` (manifest + motions): load the `.mod` of the same name, the
        // sandbox finds the sibling `.chr` for the motions.
        let body_entry = body_entry.map(|entry| entry.replace(".chr", ".mod"));
        let body = body_entry.and_then(|entry| {
            eprintln!(
                "outfit: class {} armor {} \"{}\" -> Character/{entry}",
                self.class,
                self.armor,
                name_of(self.armor)
            );
            ActorModel::load(
                ttb_path,
                "Character",
                &entry,
                tile_size,
                "CORUM_PLAYER_ANIM",
            )
            .map_err(|error| eprintln!("outfit: body {entry}: {error}"))
            .ok()
        });
        let mut parts = Vec::new();
        let Some(rig) = body.as_ref().and_then(|body| body.rig.as_ref()) else {
            return (body, parts);
        };
        // Heads: `RESTYPE_HEAD_MALE` for classes below 4, `_FEMALE` otherwise; the id `1001`
        // names `ph101001.mod` (`ph2...` for the female heads).
        if self.head != 0 {
            let entry = head_entry(self.class, self.head);
            parts.extend(Attachment::load(
                ttb_path,
                rig,
                "Bip01 Head",
                "head",
                &["Character"],
                &entry,
                tile_size,
            ));
        }
        let item_parts = [
            (self.helmet, "Bip01 Head", "helmet", "CORUM_HELMET_MODEL"),
            (self.right, "Bip01 R Hand", "right hand", "CORUM_ITEM_MODEL"),
            (self.left, "Bip01 L Hand", "left hand", "CORUM_SHIELD_MODEL"),
        ];
        for (id, default_bone, label, model_variable) in item_parts {
            if id == 0 {
                continue;
            }
            let bone = if label == "right hand" {
                env::var("CORUM_ITEM_BONE").unwrap_or_else(|_| default_bone.to_owned())
            } else {
                default_bone.to_owned()
            };
            let label = format!("{label} item {id} \"{}\"", name_of(id));
            let Some(entry) = catalog.as_ref().and_then(|catalog| {
                catalog.model_entry(id, env_number(model_variable).unwrap_or(0))
            }) else {
                eprintln!("{label}: no 3D model");
                continue;
            };
            parts.extend(Attachment::load(
                ttb_path,
                rig,
                &bone,
                &label,
                &["Item", "Character"],
                &entry,
                tile_size,
            ));
        }
        (body, parts)
    }
}

/// One model the map script places many times, with every instance already in world space.
struct PropBatch {
    vertices: Vec<Vertex>,
    textures: MapTextures,
    /// Set for animated `.CHR` props (fire, flags, machinery): the posed model and where it stands.
    animated: Option<AnimatedProp>,
}

struct AnimatedProp {
    model: ActorModel,
    instances: Vec<MapObject>,
}

/// Animated props are re-posed and re-uploaded every frame, so only batches up to this many
/// vertices (all instances together) are animated; bigger ones stay in the bind pose.
const MAX_ANIMATED_PROP_VERTICES: usize = 60_000;

/// Direction of the `GX_OBJECT` rotation. The script angle is a Direct3D (left-handed) rotation
/// about Y; the world is drawn with the raw coordinates, so glam's right-handed rotation needs
/// the opposite sign to keep props aligned with the STM scenery.
const PROP_ROTATION_SIGN: f32 = -1.0;

/// Places the objects listed by the `.map` script (`GX_OBJECT`): `.MOD` models and `.CHR`
/// manifests (animated props, drawn in bind pose). Models come from `Map_chr.pak`.
fn load_props(ttb_path: &Path, map: &TileMap) -> Vec<PropBatch> {
    let Ok(bytes) = fs::read(ttb_path.with_extension("map")) else {
        return Vec::new();
    };
    let Ok(script) = MapScript::parse(&bytes) else {
        return Vec::new();
    };
    if script.objects.is_empty() {
        return Vec::new();
    }
    let archive = match open_data_package(ttb_path, "Map_chr") {
        Ok(archive) => archive,
        Err(error) => {
            eprintln!("props unavailable: {error}");
            return Vec::new();
        }
    };
    // Prop textures are mostly in Map_chr itself; a few live with the map textures.
    let mut packages = TexturePackages {
        dds: vec![archive.clone()],
        tif: vec![archive.clone()],
    };
    packages.dds.extend(open_data_package(ttb_path, "Map_dds"));
    packages.tif.extend(open_data_package(ttb_path, "Map_tif"));

    let mut by_resource: BTreeMap<String, Vec<&MapObject>> = BTreeMap::new();
    for object in &script.objects {
        by_resource
            .entry(object.resource.to_ascii_lowercase())
            .or_default()
            .push(object);
    }

    let tile_size = map.tile_size as f32;
    let (mut placed, mut unresolved, mut broken) = (0_usize, 0_usize, 0_usize);
    let mut batches = Vec::new();
    for (resource, instances) in &by_resource {
        // A `.CHR` is a manifest that names the model and its animations.
        let model_name = if resource.ends_with(".chr") {
            archive
                .read_entry(resource)
                .ok()
                .and_then(|bytes| ChrManifest::parse(&bytes).ok())
                .map(|manifest| manifest.model_file)
        } else {
            Some(resource.clone())
        };
        let Some(bytes) = model_name.and_then(|name| archive.read_entry(&name).ok()) else {
            unresolved += instances.len();
            continue;
        };
        let is_chr = resource.ends_with(".chr");
        let model = match ActorModel::from_bytes(&bytes, &packages, tile_size, is_chr) {
            Ok(model) => model,
            Err(_) => {
                broken += instances.len();
                continue;
            }
        };
        let mut model = model;
        let total = model.vertices.len() * instances.len();
        if is_chr && total <= MAX_ANIMATED_PROP_VERTICES {
            // Play the first real motion of the manifest, looping.
            // `attach_motions` reads `<stem>.chr`, which is the resource itself.
            model.attach_motions(&archive, resource, Some(0), Some(0));
            let has_motion = model
                .rig
                .as_ref()
                .is_some_and(|rig| rig.motions.iter().flatten().next().is_some());
            if has_motion && let Some(rig) = &mut model.rig {
                rig.idle = rig.motions.iter().position(Option::is_some);
                rig.moving = rig.idle;
            } else {
                model.rig = None;
            }
        } else {
            model.rig = None;
        }
        let mut vertices = Vec::with_capacity(total);
        for object in instances {
            placed += 1;
            append_instance(&mut vertices, &model.vertices, object, map);
        }
        let animated = model.rig.is_some().then(|| AnimatedProp {
            instances: instances.iter().map(|object| (*object).clone()).collect(),
            model: ActorModel {
                facing: 0.0,
                vertices: model.vertices.clone(),
                textures: MapTextures::default(),
                rig: model.rig.take(),
            },
        });
        batches.push(PropBatch {
            vertices,
            textures: model.textures,
            animated,
        });
    }
    eprintln!(
        "placed {placed} of {} map objects ({} models; {unresolved} without a model file, {broken} undecodable)",
        script.objects.len(),
        batches.len()
    );
    batches
}

/// Appends one instance of `local` (tile units, origin at the model origin) to `output`, applying
/// the object's scale, rotation and position. Negative scales mirror the model; the winding
/// flips with them, which does not matter because nothing is culled.
fn append_instance(output: &mut Vec<Vertex>, local: &[Vertex], object: &MapObject, map: &TileMap) {
    let scale = Vec3::from_array(object.scale);
    let axis = Vec3::from_array(object.axis).normalize_or(Vec3::Y);
    if !scale.is_finite() || !object.angle_radians.is_finite() {
        return;
    }
    let rotation = glam::Quat::from_axis_angle(axis, PROP_ROTATION_SIGN * object.angle_radians);
    let units = 1.0 / map.tile_size as f32;
    let translation = Vec3::new(
        object.position[0] * units - map.width as f32 * 0.5,
        object.position[1] * units,
        object.position[2] * units - map.height as f32 * 0.5,
    );
    // Normals transform with the inverse scale, then rotate.
    let inverse_scale = Vec3::new(
        1.0 / scale.x.abs().max(1e-4) * scale.x.signum(),
        1.0 / scale.y.abs().max(1e-4) * scale.y.signum(),
        1.0 / scale.z.abs().max(1e-4) * scale.z.signum(),
    );
    output.extend(local.iter().map(|vertex| {
        let mut placed = *vertex;
        placed.position =
            (rotation * (Vec3::from_array(vertex.position) * scale) + translation).to_array();
        placed.normal = (rotation * (Vec3::from_array(vertex.normal) * inverse_scale))
            .normalize_or(Vec3::Y)
            .to_array();
        placed
    }));
}

/// GPU side of an [`ActorModel`]: its own texture array and a vertex buffer rewritten each frame.
struct ActorGpu {
    kind: ActorKind,
    bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    count: u32,
}

/// GPU side of a [`PropBatch`]: static, uploaded once.
struct PropGpu {
    bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    count: u32,
}

/// The texture packages a map can draw from.
struct TexturePackages {
    dds: Vec<PakArchive>,
    tif: Vec<PakArchive>,
}

/// Map textures resolved from the `Map_dds` and `Map_tif` packages, one array layer per texture.
#[derive(Default)]
struct MapTextures {
    layer_size: u32,
    images: Vec<DecodedImage>,
    /// Per image: mostly partial alpha (water, glass), so it is alpha-blended.
    translucent: Vec<bool>,
    /// Per image: an effect on a black background, so it is additive.
    black_keyed: Vec<bool>,
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
                .into_iter()
                .collect(),
            tif: open_data_package(ttb_path, "Map_tif")
                .map_err(|error| eprintln!("TIFF textures unavailable: {error}"))
                .into_iter()
                .collect(),
        };

        let names: Vec<String> = model
            .materials
            .iter()
            .map(|material| material.texture_name.clone())
            .collect();
        let textures = Self::from_names(&names, &packages);
        eprintln!(
            "loaded {} unique textures for {} materials",
            textures.images.len(),
            names.len()
        );
        textures
    }

    /// One layer per unique texture name (case-insensitive); materials without a texture
    /// keep `None` and use a flat colour.
    fn from_names(names: &[String], packages: &TexturePackages) -> Self {
        let mut textures = Self::default();
        let mut layer_by_name: Vec<(String, Option<u32>)> = Vec::new();
        for name in names {
            let key = name.to_ascii_lowercase();
            if let Some((_, layer)) = layer_by_name.iter().find(|(known, _)| *known == key) {
                textures.material_layers.push(*layer);
                continue;
            }
            let layer = textures.decode(packages, name);
            layer_by_name.push((key, layer));
            textures.material_layers.push(layer);
        }
        textures.layer_size = textures
            .images
            .iter()
            .map(|image| image.width.max(image.height))
            .max()
            .unwrap_or(1);
        textures
    }

    /// Materials name a `.tga`, but the packages hold the same texture as `.dds` (96% of the
    /// 196 packed maps' materials) or, for the rest, as an uncompressed `.tif`.
    fn decode(&mut self, packages: &TexturePackages, texture_name: &str) -> Option<u32> {
        let (stem, extension) = texture_name
            .rsplit_once('.')
            .unwrap_or((texture_name, "tga"));
        let dds = format!("{stem}.dds");
        let tif = format!("{stem}.tif");
        let read = |archives: &[PakArchive], entry: &str| {
            archives
                .iter()
                .find_map(|archive| archive.read_entry(entry).ok())
        };
        let from_dds = || {
            read(&packages.dds, &dds).map(|bytes| {
                DecodedImage::from_dds(&bytes).map_err(|error| format!("texture '{dds}': {error}"))
            })
        };
        let from_tif = || {
            read(&packages.tif, &tif).map(|bytes| {
                DecodedImage::from_tiff(&bytes)
                    .map(flipped_vertically)
                    .map_err(|error| format!("texture '{tif}': {error}"))
            })
        };
        // The extension the material asks for is authoritative: a name can exist as both a
        // `.dds` and a different `.tif` (`ks-tree02`), and a `.tif` request means the TIFF.
        // Other extensions (`.tga`) are stored in the packages as `.dds`.
        let decoded = if extension.eq_ignore_ascii_case("tif") {
            from_tif().or_else(from_dds)
        } else {
            from_dds().or_else(from_tif)
        }?;
        match decoded {
            Ok(image) => {
                self.translucent.push(is_translucent(&image));
                self.black_keyed.push(is_black_keyed(&image));
                self.images.push(image);
                Some((self.images.len() - 1) as u32)
            }
            Err(message) => {
                eprintln!("{message}");
                None
            }
        }
    }

    fn is_translucent(&self, layer: u32) -> bool {
        self.translucent
            .get(layer as usize)
            .copied()
            .unwrap_or(false)
    }

    /// Blend class of a surface: additive when its material asks for it or its texture is an
    /// effect on black, alpha-blended for translucent textures, opaque (with cut-outs) otherwise.
    fn blend_class(&self, layer: Option<u32>, material_additive: bool) -> f32 {
        let black_keyed = layer.is_some_and(|layer| {
            self.black_keyed
                .get(layer as usize)
                .copied()
                .unwrap_or(false)
        });
        if material_additive || black_keyed {
            BLEND_ADDITIVE
        } else if layer.is_some_and(|layer| self.is_translucent(layer)) {
            BLEND_ALPHA
        } else {
            BLEND_OPAQUE
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

/// True for an opaque texture that is mostly black: an effect meant to be added to the frame.
fn is_black_keyed(image: &DecodedImage) -> bool {
    let texels = image.rgba.len() / 4;
    if texels == 0 {
        return false;
    }
    let opaque = image
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|t| t[3] >= 250)
        .count();
    let black = image
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|t| t[0].max(t[1]).max(t[2]) < 12)
        .count();
    opaque as f32 / texels as f32 > 0.95 && black as f32 / texels as f32 > BLACK_KEYED_FRACTION
}

/// True when most texels have a partial alpha, as water and glass do. Hard cut-outs have
/// texels that are almost all fully opaque or fully clear.
fn is_translucent(image: &DecodedImage) -> bool {
    let texels = image.rgba.len() / 4;
    if texels == 0 {
        return false;
    }
    let partial = image
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|texel| (6..=249).contains(&texel[3]))
        .count();
    partial as f32 / texels as f32 > TRANSLUCENT_PARTIAL_FRACTION
}

/// The map packages' TIFFs are stored top row first but sampled with `v = 0` at the bottom
/// (like TGA): the trunk of `ks-m2tree` uses `v` 0.05..0.26 for its bark strip, which is the
/// bottom of the file. DDS textures need no flip.
fn flipped_vertically(image: DecodedImage) -> DecodedImage {
    let row = image.width as usize * 4;
    let rgba = image
        .rgba
        .chunks_exact(row)
        .rev()
        .flatten()
        .copied()
        .collect();
    DecodedImage { rgba, ..image }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn image(texels: &[[u8; 4]]) -> DecodedImage {
        DecodedImage {
            width: texels.len() as u32,
            height: 1,
            rgba: texels.iter().flatten().copied().collect(),
        }
    }

    #[test]
    fn the_cursor_ray_lands_on_the_ground_under_the_pixel() {
        let camera = Camera {
            target: Vec3::new(3.0, 0.7, -2.0),
            yaw: 0.6,
            pitch: 0.9,
            distance: 9.0,
            dragging: false,
            drag_distance: 0.0,
            last_cursor: None,
        };
        let view_projection = camera.uniform(1.4).view_projection;
        let matrix = Mat4::from_cols_array_2d(&view_projection);
        for ndc in [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.6, 0.4),
            Vec2::new(-0.8, -0.5),
        ] {
            let hit =
                unproject_to_ground(view_projection, ndc, 0.0).expect("looking down at the ground");
            assert!(hit.y.abs() < 1e-3, "{hit:?}");
            // Projecting the hit point again must give the same pixel back.
            let clip = matrix * hit.extend(1.0);
            let back = Vec2::new(clip.x / clip.w, clip.y / clip.w);
            assert!(
                (back - ndc).length() < 1e-3,
                "{ndc:?} -> {hit:?} -> {back:?}"
            );
        }
        // A ray that looks at the sky never meets the ground.
        assert!(unproject_to_ground(view_projection, Vec2::new(0.0, 1.0), 40.0).is_none());
    }

    #[test]
    fn character_models_are_turned_to_face_their_movement() {
        // Sem esse giro os personagens andavam de costas (o "moonwalk").
        assert!((facing_offset("Character") - std::f32::consts::PI).abs() < 1e-6);
        assert!((facing_offset("character") - std::f32::consts::PI).abs() < 1e-6);
        assert!((facing_offset("Monster") - std::f32::consts::PI).abs() < 1e-6);
        assert_eq!(facing_offset("Npc"), 0.0);
    }

    #[test]
    fn character_parts_use_the_original_client_names() {
        // `RESTYPE_BASE_BODY`: one base body per class; heads: male below class 4, female above.
        assert_eq!(base_body_entry(1), "pm01000.mod");
        assert_eq!(base_body_entry(5), "pm05000.mod");
        assert_eq!(head_entry(1, 1001), "ph101001.mod");
        assert_eq!(head_entry(3, 1022), "ph101022.mod");
        assert_eq!(head_entry(4, 1001), "ph201001.mod");
    }

    #[test]
    fn water_like_alpha_is_translucent_but_cutouts_are_not() {
        let water = image(&[[10, 20, 30, 120]; 8]);
        assert!(is_translucent(&water));
        // A leaf: mostly fully clear or fully opaque, a little antialiasing.
        let mut leaf = vec![[0, 0, 0, 0]; 4];
        leaf.extend([[0, 90, 0, 255]; 4]);
        leaf.push([0, 90, 0, 128]);
        assert!(!is_translucent(&image(&leaf)));
    }

    #[test]
    fn opaque_mostly_black_textures_are_effects() {
        let mut fire = vec![[0, 0, 0, 255]; 8];
        fire.extend([[255, 160, 20, 255]; 2]);
        assert!(is_black_keyed(&image(&fire)));
        // A dark wall is not mostly pure black, and an alpha cut-out is never additive.
        assert!(!is_black_keyed(&image(&[[40, 38, 36, 255]; 10])));
        assert!(!is_black_keyed(&image(&[[0, 0, 0, 0]; 10])));
    }

    #[test]
    fn blend_class_prefers_the_material_flag_then_the_texture() {
        let textures = MapTextures {
            translucent: vec![false, true, false],
            black_keyed: vec![false, false, true],
            ..MapTextures::default()
        };
        assert_eq!(textures.blend_class(Some(0), false), BLEND_OPAQUE);
        assert_eq!(textures.blend_class(Some(1), false), BLEND_ALPHA);
        assert_eq!(textures.blend_class(Some(2), false), BLEND_ADDITIVE);
        assert_eq!(textures.blend_class(Some(0), true), BLEND_ADDITIVE);
        assert_eq!(textures.blend_class(None, false), BLEND_OPAQUE);
    }

    #[test]
    fn tiff_rows_are_flipped_to_the_tga_origin() {
        let image = DecodedImage {
            width: 1,
            height: 2,
            rgba: vec![1, 1, 1, 1, 2, 2, 2, 2],
        };
        assert_eq!(flipped_vertically(image).rgba, vec![2, 2, 2, 2, 1, 1, 1, 1]);
    }
}

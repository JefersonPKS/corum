//! Interface original: janelas, sprites e áreas clicáveis, lidas das tabelas de `Data\Manager`.
//!
//! **[confirmado]** O cliente (`Interface.cpp`, `InterfaceSpr.h`) monta cada janela assim:
//!
//! - `InterfaceFrameInfo.cdb`: uma janela por registro (id, nome, largura, altura, posição inicial, tipo,
//!   índice, "ativa" e rolagem) para uma tela de 1024 × 768 (o minimapa, 159 × 186 em x = 865, encosta no
//!   canto direito). O arquivo tem 4 bytes a mais que a struct do código-fonte: a **posição** `left, top`.
//! - `InterfaceComponentInfo.cdb`: o que há dentro da janela, em coordenadas da janela. Tipo de recurso 3 =
//!   **sprite** (`resource_id` é um id de `InterfaceSpriteManager`), tipo 1 = **área de colisão**
//!   (`left, top, right, bottom` são uma caixa e `resource_id` é o `CHECKTYPE` de `Menu.h`: 1 fechar, 2 botão,
//!   3 botão de pressão, 4 rolagem, 5 mover a janela, 6 item...).
//! - `InterfaceSpriteManager.cdb`: id do sprite → id do recurso.
//! - `InterfaceResourceInfo.cdb`: arquivo de imagem (no pacote `UI`) e recorte. Como
//!   `CreateSpriteObject(arquivo, x, y, largura, altura)`, os campos `left, top, right, bottom` são na
//!   verdade **x, y, largura, altura**; o tipo 0 é a imagem inteira.
//!
//! Vários componentes no mesmo lugar são **estados** do mesmo botão (normal, sob o mouse, apertado); só o
//! primeiro entra no desenho estático. Os textos e números dinâmicos (nome, PV) não vêm das tabelas.

use crate::cdb;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

/// Resolução de referência da interface original.
pub const SCREEN_WIDTH: u32 = 1024;
pub const SCREEN_HEIGHT: u32 = 768;

/// Imagens de estado (marca de "ligado" das opções): o estado atual não vem das tabelas.
const STATE_OVERLAYS: [&str; 1] = ["checkv1.tif"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiError(String);

impl fmt::Display for UiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "interface tables: {}", self.0)
    }
}

impl std::error::Error for UiError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiFrame {
    pub id: u16,
    pub name: String,
    pub width: u16,
    pub height: u16,
    /// Posição inicial na tela de 1024 × 768.
    pub left: u16,
    pub top: u16,
    pub kind: u8,
    pub index: u16,
    /// Já aberta ao entrar no jogo.
    pub active: bool,
}

/// Um sprite a desenhar dentro de uma janela.
#[derive(Debug, Clone, PartialEq)]
pub struct UiSprite {
    /// Nome do arquivo no pacote `UI` (`menu_1.tga`).
    pub file: String,
    /// Recorte `[x, y, largura, altura]` da imagem; `None` = a imagem inteira.
    pub source: Option<[u32; 4]>,
    /// Posição dentro da janela.
    pub position: [i32; 2],
    pub scale: [f32; 2],
    /// Ordem de desenho: 0 = fundo da janela, maiores por cima.
    pub order: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitKind {
    Close,
    Button,
    PushButton,
    Scroll,
    MoveWindow,
    Item,
    Other(u16),
}

impl HitKind {
    fn from_check_type(value: u16) -> Self {
        match value {
            1 => Self::Close,
            2 => Self::Button,
            3 => Self::PushButton,
            4 => Self::Scroll,
            5 => Self::MoveWindow,
            6 => Self::Item,
            other => Self::Other(other),
        }
    }
}

/// Área clicável de uma janela, em coordenadas da janela.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UiHitBox {
    pub kind: HitKind,
    pub callback: u8,
    /// `[esquerda, topo, direita, baixo]`.
    pub rect: [i32; 4],
}

impl UiHitBox {
    #[must_use]
    pub fn contains(&self, point: [i32; 2]) -> bool {
        point[0] >= self.rect[0]
            && point[0] < self.rect[2]
            && point[1] >= self.rect[1]
            && point[1] < self.rect[3]
    }
}

#[derive(Debug, Clone)]
struct Resource {
    file: String,
    whole_image: bool,
    rect: [u32; 4],
}

#[derive(Debug, Clone)]
struct Component {
    window: u16,
    callback: u8,
    resource_type: u8,
    resource: u16,
    order: u8,
    rect: [u16; 4],
    scale: [f32; 2],
}

#[derive(Debug, Clone, Default)]
pub struct UiCatalog {
    frames: BTreeMap<u16, UiFrame>,
    resources: BTreeMap<u16, Resource>,
    sprites: BTreeMap<u16, u16>,
    components: Vec<Component>,
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn f32_at(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn text_at(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Divide um corpo em registros de `size` bytes, ou erra se não fechar.
fn records<'a>(name: &str, body: &'a [u8], size: usize) -> Result<Vec<&'a [u8]>, UiError> {
    if !body.len().is_multiple_of(size) {
        return Err(UiError(format!(
            "{name}: {} bytes is not a multiple of {size}",
            body.len()
        )));
    }
    // `size` só é conhecido em tempo de execução.
    #[allow(clippy::chunks_exact_to_as_chunks)]
    Ok(body.chunks_exact(size).collect())
}

impl UiCatalog {
    /// Lê as quatro tabelas de interface de uma pasta `Data\Manager`.
    pub fn load(manager: &Path) -> Result<Self, UiError> {
        let read = |name: &str| {
            let path = manager.join(format!("{name}.cdb"));
            std::fs::read(&path)
                .map_err(|error| UiError(format!("{}: {error}", path.display())))
                .and_then(|bytes| {
                    cdb::decode(&bytes).map_err(|error| UiError(format!("{name}: {error}")))
                })
        };
        Self::from_tables(
            &read("InterfaceFrameInfo")?,
            &read("InterfaceResourceInfo")?,
            &read("InterfaceSpriteManager")?,
            &read("InterfaceComponentInfo")?,
        )
    }

    /// Monta o catálogo a partir dos quatro corpos já decifrados.
    pub fn from_tables(
        frames: &[u8],
        resources: &[u8],
        sprites: &[u8],
        components: &[u8],
    ) -> Result<Self, UiError> {
        let mut catalog = Self::default();
        // 45 B: id u16, nome[30], largura, altura, esquerda, topo (u16), tipo u8, índice u16, ativa u8, rolagem u8.
        for record in records("InterfaceFrameInfo", frames, 45)? {
            let id = u16_at(record, 0);
            catalog.frames.insert(
                id,
                UiFrame {
                    id,
                    name: text_at(&record[2..32]),
                    width: u16_at(record, 32),
                    height: u16_at(record, 34),
                    left: u16_at(record, 36),
                    top: u16_at(record, 38),
                    kind: record[40],
                    index: u16_at(record, 41),
                    active: record[43] != 0,
                },
            );
        }
        // 61 B: id u16, arquivo[50], tipo u8, x, y, largura, altura (u16).
        for record in records("InterfaceResourceInfo", resources, 61)? {
            catalog.resources.insert(
                u16_at(record, 0),
                Resource {
                    file: text_at(&record[2..52]),
                    whole_image: record[52] == 0,
                    rect: [
                        u32::from(u16_at(record, 53)),
                        u32::from(u16_at(record, 55)),
                        u32::from(u16_at(record, 57)),
                        u32::from(u16_at(record, 59)),
                    ],
                },
            );
        }
        // 4 B: id u16, id do recurso u16.
        for record in records("InterfaceSpriteManager", sprites, 4)? {
            catalog.sprites.insert(u16_at(record, 0), u16_at(record, 2));
        }
        // 25 B: janela u16, chamada u8, tipo de recurso u8, recurso u16, ordem u8, esquerda, topo, direita,
        // baixo (u16), escala x, y (f32), valor u8, posição u8.
        for record in records("InterfaceComponentInfo", components, 25)? {
            catalog.components.push(Component {
                window: u16_at(record, 0),
                callback: record[2],
                resource_type: record[3],
                resource: u16_at(record, 4),
                order: record[6],
                rect: [
                    u16_at(record, 7),
                    u16_at(record, 9),
                    u16_at(record, 11),
                    u16_at(record, 13),
                ],
                scale: [f32_at(record, 15), f32_at(record, 19)],
            });
        }
        Ok(catalog)
    }

    #[must_use]
    pub fn frame(&self, id: u16) -> Option<&UiFrame> {
        self.frames.get(&id)
    }

    /// Id da janela pelo nome (`ITEM`, `CHAR`, `SKILL`, `GAMEMENU`...), sem diferenciar maiúsculas.
    #[must_use]
    pub fn window_named(&self, name: &str) -> Option<u16> {
        self.frames
            .values()
            .find(|frame| frame.name.eq_ignore_ascii_case(name))
            .map(|frame| frame.id)
    }

    pub fn frames(&self) -> impl Iterator<Item = &UiFrame> {
        self.frames.values()
    }

    /// Sprites da janela, do fundo para a frente. Estados do mesmo botão (mesma ordem e posição) e
    /// as marcas de opção ligada não entram.
    #[must_use]
    pub fn window_sprites(&self, window: u16) -> Vec<UiSprite> {
        let mut parts: Vec<&Component> = self
            .components
            .iter()
            .filter(|component| component.window == window && component.resource_type == 3)
            .collect();
        parts.sort_by_key(|component| (component.order, component.callback));
        let mut seen = Vec::new();
        let mut sprites = Vec::new();
        for part in parts {
            let position = [i32::from(part.rect[0]), i32::from(part.rect[1])];
            if seen.contains(&(part.order, position)) {
                continue;
            }
            let Some(resource) = self
                .sprites
                .get(&part.resource)
                .and_then(|resource| self.resources.get(resource))
            else {
                continue;
            };
            if STATE_OVERLAYS
                .iter()
                .any(|name| resource.file.eq_ignore_ascii_case(name))
            {
                continue;
            }
            seen.push((part.order, position));
            sprites.push(UiSprite {
                file: resource.file.clone(),
                source: (!resource.whole_image).then_some(resource.rect),
                position,
                scale: part.scale,
                order: part.order,
            });
        }
        sprites
    }

    /// Áreas clicáveis da janela (tipo de recurso 1).
    #[must_use]
    pub fn window_hit_boxes(&self, window: u16) -> Vec<UiHitBox> {
        self.components
            .iter()
            .filter(|component| component.window == window && component.resource_type == 1)
            .map(|component| UiHitBox {
                kind: HitKind::from_check_type(component.resource),
                callback: component.callback,
                rect: component.rect.map(i32::from),
            })
            .collect()
    }
}

/// Uma janela aberta e onde ela está na tela de 1024 x 768.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenWindow {
    pub id: u16,
    pub position: [i32; 2],
}

/// Resultado de apertar o mouse sobre a interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Press {
    /// Fora de qualquer janela: o clique é do mundo (andar).
    World,
    /// Dentro de uma janela (o clique é da interface).
    Window(u16),
    /// Apertou o botão de fechar; a janela foi fechada.
    Closed(u16),
}

/// A "mesa" de janelas: quais estão abertas, a ordem (a última fica por cima) e o arraste.
#[derive(Debug, Clone)]
pub struct UiDesktop {
    catalog: UiCatalog,
    open: Vec<OpenWindow>,
    drag: Option<(u16, [i32; 2])>,
}

impl UiDesktop {
    #[must_use]
    pub fn new(catalog: UiCatalog) -> Self {
        Self {
            catalog,
            open: Vec::new(),
            drag: None,
        }
    }

    #[must_use]
    pub fn catalog(&self) -> &UiCatalog {
        &self.catalog
    }

    /// Janelas abertas, da mais de trás para a da frente.
    #[must_use]
    pub fn open_windows(&self) -> &[OpenWindow] {
        &self.open
    }

    #[must_use]
    pub fn is_open(&self, id: u16) -> bool {
        self.open.iter().any(|window| window.id == id)
    }

    /// Abre a janela na posição inicial da tabela (ou a traz para a frente se já estiver aberta).
    pub fn open(&mut self, id: u16) {
        let Some(frame) = self.catalog.frame(id) else {
            return;
        };
        if let Some(index) = self.open.iter().position(|window| window.id == id) {
            let window = self.open.remove(index);
            self.open.push(window);
        } else {
            self.open.push(OpenWindow {
                id,
                position: [i32::from(frame.left), i32::from(frame.top)],
            });
        }
    }

    pub fn close(&mut self, id: u16) {
        self.open.retain(|window| window.id != id);
        if self.drag.is_some_and(|(dragged, _)| dragged == id) {
            self.drag = None;
        }
    }

    /// Abre a janela se estiver fechada e fecha se estiver aberta (o atalho de teclado).
    pub fn toggle(&mut self, id: u16) {
        if self.is_open(id) {
            self.close(id);
        } else {
            self.open(id);
        }
    }

    /// Fecha a janela da frente; `false` se não havia nenhuma.
    pub fn close_top(&mut self) -> bool {
        self.drag = None;
        self.open.pop().is_some()
    }

    fn size_of(&self, id: u16) -> [i32; 2] {
        self.catalog.frame(id).map_or([0, 0], |frame| {
            [i32::from(frame.width), i32::from(frame.height)]
        })
    }

    /// Mouse apertado em `point` (coordenadas da tela de 1024 x 768).
    pub fn press(&mut self, point: [i32; 2]) -> Press {
        let hit = self.open.iter().rposition(|window| {
            let size = self.size_of(window.id);
            point[0] >= window.position[0]
                && point[0] < window.position[0] + size[0]
                && point[1] >= window.position[1]
                && point[1] < window.position[1] + size[1]
        });
        let Some(index) = hit else {
            return Press::World;
        };
        let window = self.open.remove(index);
        self.open.push(window);
        let local = [point[0] - window.position[0], point[1] - window.position[1]];
        for hit_box in self.catalog.window_hit_boxes(window.id) {
            if !hit_box.contains(local) {
                continue;
            }
            match hit_box.kind {
                HitKind::Close => {
                    self.close(window.id);
                    return Press::Closed(window.id);
                }
                HitKind::MoveWindow => self.drag = Some((window.id, local)),
                _ => {}
            }
        }
        Press::Window(window.id)
    }

    /// Mouse solto: termina um arraste.
    pub fn release(&mut self) {
        self.drag = None;
    }

    /// Mouse movido para `point` (coordenadas da tela): move a janela arrastada, sem sair da tela.
    pub fn motion(&mut self, point: [i32; 2]) {
        let Some((id, grab)) = self.drag else {
            return;
        };
        let size = self.size_of(id);
        let max_x = (SCREEN_WIDTH as i32 - size[0].min(64)).max(0);
        let max_y = (SCREEN_HEIGHT as i32 - 24).max(0);
        if let Some(window) = self.open.iter_mut().find(|window| window.id == id) {
            window.position = [
                (point[0] - grab[0]).clamp(-size[0] + 64, max_x),
                (point[1] - grab[1]).clamp(0, max_y),
            ];
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: u16, name: &str) -> Vec<u8> {
        let mut record = id.to_le_bytes().to_vec();
        let mut text = name.as_bytes().to_vec();
        text.resize(30, 0);
        record.extend(text);
        for value in [256u16, 640, 717, 29] {
            record.extend(value.to_le_bytes());
        }
        record.push(3);
        record.extend(id.to_le_bytes());
        record.extend([1, 0]);
        record
    }

    fn resource(id: u16, file: &str, kind: u8, rect: [u16; 4]) -> Vec<u8> {
        let mut record = id.to_le_bytes().to_vec();
        let mut text = file.as_bytes().to_vec();
        text.resize(50, 0);
        record.extend(text);
        record.push(kind);
        for value in rect {
            record.extend(value.to_le_bytes());
        }
        record
    }

    fn component(
        window: u16,
        callback: u8,
        kind: u8,
        resource: u16,
        order: u8,
        rect: [u16; 4],
    ) -> Vec<u8> {
        let mut record = window.to_le_bytes().to_vec();
        record.extend([callback, kind]);
        record.extend(resource.to_le_bytes());
        record.push(order);
        for value in rect {
            record.extend(value.to_le_bytes());
        }
        record.extend(1.0f32.to_le_bytes());
        record.extend(1.0f32.to_le_bytes());
        record.extend([0, 1]);
        record
    }

    fn sample() -> UiCatalog {
        let frames = frame(2, "ITEM");
        let mut resources = resource(10, "back.tif", 0, [0; 4]);
        resources.extend(resource(11, "menu_1.tga", 1, [187, 16, 13, 13]));
        resources.extend(resource(12, "checkv1.tif", 0, [0; 4]));
        let mut sprites = Vec::new();
        for (id, target) in [(1u16, 10u16), (2, 11), (3, 11), (4, 12)] {
            sprites.extend(id.to_le_bytes());
            sprites.extend(target.to_le_bytes());
        }
        let mut components = component(2, 0, 3, 1, 0, [0, 0, 0, 0]);
        components.extend(component(2, 2, 3, 2, 1, [242, 4, 0, 0]));
        components.extend(component(2, 3, 3, 3, 1, [242, 4, 0, 0])); // estado do mesmo botão
        components.extend(component(2, 9, 3, 4, 1, [50, 60, 0, 0])); // marca de opção
        components.extend(component(2, 2, 1, 1, 0, [242, 4, 256, 16]));
        components.extend(component(2, 4, 1, 4, 0, [0, 0, 200, 20]));
        UiCatalog::from_tables(&frames, &resources, &sprites, &components).unwrap()
    }

    #[test]
    fn frames_carry_size_position_and_flags() {
        let catalog = sample();
        let item = catalog.frame(2).unwrap();
        assert_eq!(
            (item.name.as_str(), item.width, item.height),
            ("ITEM", 256, 640)
        );
        assert_eq!(
            (item.left, item.top, item.kind, item.index, item.active),
            (717, 29, 3, 2, true)
        );
        assert_eq!(catalog.window_named("item"), Some(2));
        assert_eq!(catalog.window_named("nothing"), None);
    }

    #[test]
    fn sprites_use_the_crop_rect_and_skip_states_and_check_marks() {
        let sprites = sample().window_sprites(2);
        assert_eq!(sprites.len(), 2, "{sprites:?}");
        assert_eq!(sprites[0].file, "back.tif");
        assert_eq!(sprites[0].source, None);
        assert_eq!(sprites[1].file, "menu_1.tga");
        assert_eq!(sprites[1].source, Some([187, 16, 13, 13]));
        assert_eq!(sprites[1].position, [242, 4]);
    }

    #[test]
    fn hit_boxes_use_the_check_type_of_the_client() {
        let boxes = sample().window_hit_boxes(2);
        assert_eq!(boxes.len(), 2);
        assert_eq!(boxes[0].kind, HitKind::Close);
        assert!(boxes[0].contains([250, 10]));
        assert!(!boxes[0].contains([256, 10]), "the right edge is exclusive");
        assert_eq!(boxes[1].kind, HitKind::Scroll);
    }

    #[test]
    fn desktop_opens_and_leaves_the_world_clicks_alone() {
        let mut desktop = UiDesktop::new(sample());
        assert!(!desktop.is_open(2));
        desktop.toggle(2);
        assert_eq!(
            desktop.open_windows(),
            [OpenWindow {
                id: 2,
                position: [717, 29]
            }]
        );
        // Fora da janela (256 x 640 em 717, 29) o clique é do mundo; dentro é da interface.
        assert_eq!(desktop.press([10, 10]), Press::World);
        assert_eq!(desktop.press([800, 300]), Press::Window(2));
        desktop.toggle(2);
        assert!(desktop.open_windows().is_empty());
    }

    #[test]
    fn closing_and_dragging_use_the_hit_boxes() {
        // Janela 7 com barra de mover (tipo 5) e botão de fechar (tipo 1).
        let frames = frame(7, "DRAG");
        let resources = resource(10, "back.tif", 0, [0; 4]);
        let mut sprites = 1u16.to_le_bytes().to_vec();
        sprites.extend(10u16.to_le_bytes());
        let mut components = component(7, 0, 3, 1, 0, [0, 0, 0, 0]);
        components.extend(component(7, 1, 1, 5, 0, [0, 0, 200, 20]));
        components.extend(component(7, 2, 1, 1, 0, [242, 4, 256, 16]));
        let catalog = UiCatalog::from_tables(&frames, &resources, &sprites, &components).unwrap();
        let mut desktop = UiDesktop::new(catalog);
        desktop.open(7);
        // Aperta na barra de título e arrasta 50 px para a esquerda e 40 para baixo.
        assert_eq!(desktop.press([717 + 100, 29 + 10]), Press::Window(7));
        desktop.motion([717 + 100 - 50, 29 + 10 + 40]);
        assert_eq!(desktop.open_windows()[0].position, [667, 69]);
        desktop.release();
        desktop.motion([0, 0]);
        assert_eq!(
            desktop.open_windows()[0].position,
            [667, 69],
            "released: no more dragging"
        );
        // O botão de fechar (no canto) fecha e some da mesa.
        assert_eq!(desktop.press([667 + 245, 69 + 8]), Press::Closed(7));
        assert!(desktop.open_windows().is_empty());
        assert!(!desktop.close_top());
    }

    #[test]
    fn a_reopened_window_comes_to_the_front() {
        let mut two = frame(2, "A");
        two.extend(frame(3, "B")); // as duas na mesma posição inicial (717, 29)
        let resources = resource(10, "back.tif", 0, [0; 4]);
        let mut sprites = 1u16.to_le_bytes().to_vec();
        sprites.extend(10u16.to_le_bytes());
        let catalog = UiCatalog::from_tables(&two, &resources, &sprites, &[]).unwrap();
        let mut desktop = UiDesktop::new(catalog);
        desktop.open(2);
        desktop.open(3);
        assert_eq!(desktop.open_windows().last().unwrap().id, 3);
        desktop.open(2);
        assert_eq!(desktop.open_windows().last().unwrap().id, 2);
        assert!(desktop.close_top());
        assert_eq!(desktop.open_windows().len(), 1);
        assert_eq!(desktop.open_windows()[0].id, 3);
    }

    #[test]
    fn record_sizes_must_fit() {
        assert!(UiCatalog::from_tables(&[0; 44], &[], &[], &[]).is_err());
    }

    /// Com `CORUM_DATA`: as tabelas reais têm as janelas principais e sprites de todas elas.
    #[test]
    fn real_tables_describe_the_main_windows() {
        let Some(data) = std::env::var_os("CORUM_DATA") else {
            return;
        };
        let catalog = UiCatalog::load(&std::path::Path::new(&data).join("Manager")).unwrap();
        assert_eq!(catalog.frames().count(), 84);
        for name in ["ITEM", "CHAR", "SKILL", "GAMEMENU", "MINIMAP", "CHAT"] {
            assert!(catalog.window_named(name).is_some(), "{name}");
        }
        let minimap = catalog
            .frame(catalog.window_named("MINIMAP").unwrap())
            .unwrap();
        assert_eq!(
            (minimap.width, minimap.height, minimap.left, minimap.top),
            (159, 186, 865, 0)
        );
        assert_eq!(minimap.left + minimap.width, SCREEN_WIDTH as u16);
        let item = catalog.window_named("ITEM").unwrap();
        assert!(catalog.window_sprites(item).len() >= 5);
        assert!(
            catalog
                .window_hit_boxes(item)
                .iter()
                .any(|hit| hit.kind == HitKind::Close && hit.rect == [242, 4, 256, 16])
        );
    }
}

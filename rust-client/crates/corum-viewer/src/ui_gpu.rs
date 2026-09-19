//! Desenho 2D da interface original: sprites das janelas de `corum_assets::ui` sobre a cena.
//!
//! A interface foi feita para 1024 x 768. Ela é escalada por igual até caber na janela e centralizada
//! (`layout`), então o que sobra fica nas laterais ou em cima e embaixo.

use std::collections::HashMap;
use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use corum_assets::PakArchive;
use corum_assets::dds::DecodedImage;
use corum_assets::ui::{SCREEN_HEIGHT, SCREEN_WIDTH, UiDesktop};
use wgpu::util::DeviceExt;

/// Sprites por quadro (seis vértices cada); o resto é ignorado.
const MAX_SPRITES: usize = 4000;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UiVertex {
    position: [f32; 2],
    uv: [f32; 2],
}

struct UiTexture {
    bind_group: wgpu::BindGroup,
    size: [f32; 2],
}

pub struct UiGpu {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    vertex_buffer: wgpu::Buffer,
    /// Uma entrada por arquivo pedido; `None` quando a imagem não existe ou não decodifica.
    textures: HashMap<String, Option<UiTexture>>,
    archive: Option<PakArchive>,
    pub desktop: UiDesktop,
    draws: Vec<(String, Range<u32>)>,
}

/// Escala e deslocamento (em pixels) que levam a tela de 1024 x 768 para a janela.
#[must_use]
pub fn layout(window: (f32, f32)) -> (f32, [f32; 2]) {
    let scale = (window.0 / SCREEN_WIDTH as f32).min(window.1 / SCREEN_HEIGHT as f32);
    let scale = scale.max(0.01);
    (
        scale,
        [
            (window.0 - SCREEN_WIDTH as f32 * scale) * 0.5,
            (window.1 - SCREEN_HEIGHT as f32 * scale) * 0.5,
        ],
    )
}

/// Posição do cursor (pixels da janela) na tela de 1024 x 768 da interface.
#[must_use]
pub fn to_screen(window: (f32, f32), cursor: (f64, f64)) -> [i32; 2] {
    let (scale, offset) = layout(window);
    [
        ((cursor.0 as f32 - offset[0]) / scale).floor() as i32,
        ((cursor.1 as f32 - offset[1]) / scale).floor() as i32,
    ]
}

/// A textura pedida pelas tabelas, ou outra extensão do mesmo nome (o cliente guarda o mesmo desenho
/// como `.tga`, `.tif` ou `.dds`).
fn load_image(archive: &PakArchive, file: &str) -> Option<DecodedImage> {
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    let mut candidates = vec![file.to_owned()];
    candidates.extend(["tga", "dds", "tif"].map(|extension| format!("{stem}.{extension}")));
    for candidate in candidates {
        let Ok(bytes) = archive.read_entry(&candidate) else {
            continue;
        };
        let extension = candidate
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let decoded = match extension.as_str() {
            "tga" => DecodedImage::from_tga(&bytes).map_err(|error| error.to_string()),
            // Ao contrário dos materiais do mapa, os TIFF da interface já saem na orientação certa.
            "tif" => DecodedImage::from_tiff(&bytes).map_err(|error| error.to_string()),
            _ => DecodedImage::from_dds(&bytes).map_err(|error| error.to_string()),
        };
        match decoded {
            Ok(image) => return Some(image),
            Err(error) => eprintln!("ui: {candidate}: {error}"),
        }
    }
    None
}

impl UiGpu {
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        archive: Option<PakArchive>,
        desktop: UiDesktop,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ui texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ui sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ui shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ui_shader.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<UiVertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            // A interface fica por cima da cena: não lê nem escreve a profundidade.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ui vertices"),
            size: (MAX_SPRITES * 6 * std::mem::size_of::<UiVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            pipeline,
            layout,
            sampler,
            vertex_buffer,
            textures: HashMap::new(),
            archive,
            desktop,
            draws: Vec::new(),
        }
    }

    /// Cria (uma vez) a textura de um arquivo do pacote `UI`.
    fn ensure_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, file: &str) {
        let key = file.to_ascii_lowercase();
        if self.textures.contains_key(&key) {
            return;
        }
        let texture = self
            .archive
            .as_ref()
            .and_then(|archive| load_image(archive, file))
            .map(|image| {
                let texture = device.create_texture_with_data(
                    queue,
                    &wgpu::TextureDescriptor {
                        label: Some("ui texture"),
                        size: wgpu::Extent3d {
                            width: image.width,
                            height: image.height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        // Espaço "gamma", como o resto do sandbox (o Direct3D 8 não converte sRGB).
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING,
                        view_formats: &[],
                    },
                    wgpu::util::TextureDataOrder::LayerMajor,
                    &image.rgba,
                );
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                UiTexture {
                    bind_group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("ui texture bind group"),
                        layout: &self.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.sampler),
                            },
                        ],
                    }),
                    size: [image.width as f32, image.height as f32],
                }
            });
        if texture.is_none() {
            eprintln!("ui: image {file} is not available");
        }
        self.textures.insert(key, texture);
    }

    /// Monta os vértices das janelas abertas para o quadro (da de trás para a da frente).
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, window: (u32, u32)) {
        self.draws.clear();
        let (width, height) = (window.0 as f32, window.1 as f32);
        if width < 1.0 || height < 1.0 || self.desktop.open_windows().is_empty() {
            return;
        }
        let (scale, offset) = layout((width, height));
        let open = self.desktop.open_windows().to_vec();
        let mut vertices: Vec<UiVertex> = Vec::new();
        for window in open {
            let sprites = self.desktop.catalog().window_sprites(window.id);
            for sprite in sprites {
                if vertices.len() >= MAX_SPRITES * 6 {
                    break;
                }
                self.ensure_texture(device, queue, &sprite.file);
                let Some(Some(texture)) = self.textures.get(&sprite.file.to_ascii_lowercase())
                else {
                    continue;
                };
                let source = sprite
                    .source
                    .map_or([0.0, 0.0, texture.size[0], texture.size[1]], |rect| {
                        rect.map(|value| value as f32)
                    });
                let size = [source[2] * sprite.scale[0], source[3] * sprite.scale[1]];
                let origin = [
                    (window.position[0] + sprite.position[0]) as f32,
                    (window.position[1] + sprite.position[1]) as f32,
                ];
                let to_ndc = |x: f32, y: f32| {
                    [
                        (offset[0] + x * scale) / width * 2.0 - 1.0,
                        1.0 - (offset[1] + y * scale) / height * 2.0,
                    ]
                };
                let (u0, v0) = (source[0] / texture.size[0], source[1] / texture.size[1]);
                let (u1, v1) = (
                    (source[0] + source[2]) / texture.size[0],
                    (source[1] + source[3]) / texture.size[1],
                );
                let corner = |x: f32, y: f32, u: f32, v: f32| UiVertex {
                    position: to_ndc(origin[0] + x, origin[1] + y),
                    uv: [u, v],
                };
                let quad = [
                    corner(0.0, 0.0, u0, v0),
                    corner(size[0], 0.0, u1, v0),
                    corner(size[0], size[1], u1, v1),
                    corner(0.0, 0.0, u0, v0),
                    corner(size[0], size[1], u1, v1),
                    corner(0.0, size[1], u0, v1),
                ];
                let start = vertices.len() as u32;
                vertices.extend(quad);
                let end = vertices.len() as u32;
                match self.draws.last_mut() {
                    Some((file, range)) if file.eq_ignore_ascii_case(&sprite.file) => {
                        range.end = end;
                    }
                    _ => self.draws.push((sprite.file.clone(), start..end)),
                }
            }
        }
        if !vertices.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        }
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        if self.draws.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        for (file, range) in &self.draws {
            let Some(Some(texture)) = self.textures.get(&file.to_ascii_lowercase()) else {
                continue;
            };
            pass.set_bind_group(0, &texture.bind_group, &[]);
            pass.draw(range.clone(), 0..1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_fits_and_centres_the_reference_screen() {
        // Janela do mesmo tamanho: sem escala nem deslocamento.
        assert_eq!(layout((1024.0, 768.0)), (1.0, [0.0, 0.0]));
        // Janela mais larga: escala pela altura e sobra nas laterais, dos dois lados igualmente.
        let (scale, offset) = layout((1536.0, 768.0));
        assert_eq!(scale, 1.0);
        assert_eq!(offset, [256.0, 0.0]);
        // Mais alta: escala pela largura.
        let (scale, offset) = layout((512.0, 600.0));
        assert_eq!(scale, 0.5);
        assert_eq!(offset, [0.0, 108.0]);
    }

    #[test]
    fn cursor_positions_map_back_to_the_reference_screen() {
        assert_eq!(to_screen((1024.0, 768.0), (500.0, 300.0)), [500, 300]);
        // Com barras laterais de 256 px, o meio da janela é o meio da tela de 1024.
        assert_eq!(to_screen((1536.0, 768.0), (768.0, 384.0)), [512, 384]);
        assert_eq!(to_screen((512.0, 384.0), (256.0, 192.0)), [512, 384]);
    }
}

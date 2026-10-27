//! A small CPU rasteriser for egui meshes.
//!
//! egui already anti-aliases through feathered vertices, so plain
//! (non-multisampled) triangle filling with premultiplied alpha blending
//! gives the same visual result as the GPU backends. Rendering into a CPU
//! buffer keeps the overlay independent of any graphics API.

use std::collections::HashMap;

use egui::epaint::{ClippedPrimitive, Primitive, Vertex};
use egui::{Color32, ImageData, Pos2, TextureFilter, TextureId, TexturesDelta};

struct Texture {
    width: usize,
    height: usize,
    pixels: Vec<Color32>,
    nearest: bool,
}

impl Texture {
    #[inline]
    fn texel(&self, x: usize, y: usize) -> [f32; 4] {
        let c = self.pixels[y * self.width + x];
        [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32]
    }

    /// Samples at normalised `uv` (premultiplied, 0..255 range).
    fn sample(&self, u: f32, v: f32) -> [f32; 4] {
        let w = self.width as f32;
        let h = self.height as f32;
        if self.nearest {
            let x = ((u * w) as isize).clamp(0, self.width as isize - 1) as usize;
            let y = ((v * h) as isize).clamp(0, self.height as isize - 1) as usize;
            return self.texel(x, y);
        }
        let fx = (u * w - 0.5).clamp(0.0, w - 1.0);
        let fy = (v * h - 0.5).clamp(0.0, h - 1.0);
        let x0 = fx as usize;
        let y0 = fy as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let tx = fx - x0 as f32;
        let ty = fy - y0 as f32;
        let a = self.texel(x0, y0);
        let b = self.texel(x1, y0);
        let c = self.texel(x0, y1);
        let d = self.texel(x1, y1);
        let mut out = [0.0; 4];
        for i in 0..4 {
            let top = a[i] + (b[i] - a[i]) * tx;
            let bottom = c[i] + (d[i] - c[i]) * tx;
            out[i] = top + (bottom - top) * ty;
        }
        out
    }
}

/// Renders egui output into an RGBA8 buffer.
pub struct SoftwareRenderer {
    width: usize,
    height: usize,
    /// RGBA8, rows top to bottom.
    pixels: Vec<u8>,
    textures: HashMap<TextureId, Texture>,
    background: Color32,
}

impl SoftwareRenderer {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; width * height * 4],
            textures: HashMap::new(),
            background: Color32::from_gray(27),
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn pixels_mut(&mut self) -> &mut [u8] {
        &mut self.pixels
    }

    pub fn update_textures(&mut self, delta: &TexturesDelta) {
        for (id, image_delta) in &delta.set {
            let (size, pixels): ([usize; 2], Vec<Color32>) = match &image_delta.image {
                ImageData::Color(img) => (img.size, img.pixels.clone()),
                ImageData::Font(font) => (font.size, font.srgba_pixels(None).collect()),
            };
            let nearest = image_delta.options.magnification == TextureFilter::Nearest;
            match image_delta.pos {
                None => {
                    self.textures.insert(
                        *id,
                        Texture {
                            width: size[0],
                            height: size[1],
                            pixels,
                            nearest,
                        },
                    );
                }
                Some([px, py]) => {
                    let Some(tex) = self.textures.get_mut(id) else {
                        continue;
                    };
                    for row in 0..size[1] {
                        let dst_y = py + row;
                        if dst_y >= tex.height {
                            break;
                        }
                        let n = size[0].min(tex.width.saturating_sub(px));
                        let dst = dst_y * tex.width + px;
                        let src = row * size[0];
                        tex.pixels[dst..dst + n].copy_from_slice(&pixels[src..src + n]);
                    }
                }
            }
        }
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    pub fn render(&mut self, primitives: &[ClippedPrimitive], pixels_per_point: f32) {
        let bg = self.background;
        for px in self.pixels.as_chunks_mut::<4>().0 {
            *px = [bg.r(), bg.g(), bg.b(), 255];
        }
        for prim in primitives {
            let Primitive::Mesh(mesh) = &prim.primitive else {
                continue;
            };
            let Some(texture) = self.textures.get(&mesh.texture_id) else {
                continue;
            };
            let clip = prim.clip_rect;
            let clip_x0 =
                ((clip.min.x * pixels_per_point).round().max(0.0) as usize).min(self.width);
            let clip_y0 =
                ((clip.min.y * pixels_per_point).round().max(0.0) as usize).min(self.height);
            let clip_x1 =
                ((clip.max.x * pixels_per_point).round().max(0.0) as usize).min(self.width);
            let clip_y1 =
                ((clip.max.y * pixels_per_point).round().max(0.0) as usize).min(self.height);
            if clip_x0 >= clip_x1 || clip_y0 >= clip_y1 {
                continue;
            }
            let clip_px = [clip_x0, clip_y0, clip_x1, clip_y1];
            for tri in mesh.indices.as_chunks::<3>().0 {
                let v = [
                    &mesh.vertices[tri[0] as usize],
                    &mesh.vertices[tri[1] as usize],
                    &mesh.vertices[tri[2] as usize],
                ];
                fill_triangle(
                    &mut self.pixels,
                    self.width,
                    clip_px,
                    texture,
                    v,
                    pixels_per_point,
                );
            }
        }
    }
}

#[inline]
fn color_f(c: Color32) -> [f32; 4] {
    [c.r() as f32, c.g() as f32, c.b() as f32, c.a() as f32]
}

#[inline]
fn blend(dst: &mut [u8], src: [f32; 4]) {
    let a = src[3].clamp(0.0, 255.0);
    if a <= 0.5 {
        return;
    }
    let inv = 1.0 - a / 255.0;
    for i in 0..4 {
        dst[i] = (src[i] + dst[i] as f32 * inv).clamp(0.0, 255.0) as u8;
    }
}

#[inline]
fn modulate(color: [f32; 4], tex: [f32; 4]) -> [f32; 4] {
    [
        color[0] * tex[0] / 255.0,
        color[1] * tex[1] / 255.0,
        color[2] * tex[2] / 255.0,
        color[3] * tex[3] / 255.0,
    ]
}

fn fill_triangle(
    pixels: &mut [u8],
    width: usize,
    clip: [usize; 4],
    texture: &Texture,
    v: [&Vertex; 3],
    ppp: f32,
) {
    let p: [Pos2; 3] = [v[0].pos * ppp, v[1].pos * ppp, v[2].pos * ppp];
    let area = (p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[1].y - p[0].y) * (p[2].x - p[0].x);
    if area.abs() < 1e-6 {
        return;
    }
    let inv_area = 1.0 / area;

    let min_x = p[0].x.min(p[1].x).min(p[2].x).floor().max(clip[0] as f32) as usize;
    let max_x = (p[0].x.max(p[1].x).max(p[2].x).ceil() as usize).min(clip[2]);
    let min_y = p[0].y.min(p[1].y).min(p[2].y).floor().max(clip[1] as f32) as usize;
    let max_y = (p[0].y.max(p[1].y).max(p[2].y).ceil() as usize).min(clip[3]);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    // Constant colour (no gradient, single texel): skip per pixel interpolation.
    let uniform = v[0].color == v[1].color
        && v[1].color == v[2].color
        && v[0].uv == v[1].uv
        && v[1].uv == v[2].uv;
    let uniform_color =
        uniform.then(|| modulate(color_f(v[0].color), texture.sample(v[0].uv.x, v[0].uv.y)));
    if let Some(c) = uniform_color {
        if c[3] <= 0.5 {
            return;
        }
    }

    let c = [
        color_f(v[0].color),
        color_f(v[1].color),
        color_f(v[2].color),
    ];

    // Barycentric weights w0 (for v0) and w1 (for v1) as edge functions,
    // stepped incrementally along x.
    let edge = |a: Pos2, b: Pos2, x: f32, y: f32| (b.x - a.x) * (y - a.y) - (b.y - a.y) * (x - a.x);
    let dw0_dx = -(p[2].y - p[1].y) * inv_area;
    let dw1_dx = -(p[0].y - p[2].y) * inv_area;
    const EPS: f32 = -1e-5;

    for y in min_y..max_y {
        let py = y as f32 + 0.5;
        let px0 = min_x as f32 + 0.5;
        let mut w0 = edge(p[1], p[2], px0, py) * inv_area;
        let mut w1 = edge(p[2], p[0], px0, py) * inv_area;
        let row = y * width;
        for x in min_x..max_x {
            let w2 = 1.0 - w0 - w1;
            if w0 >= EPS && w1 >= EPS && w2 >= EPS {
                let idx = (row + x) * 4;
                let dst = &mut pixels[idx..idx + 4];
                let src = match uniform_color {
                    Some(col) => col,
                    None => {
                        let mut col = [0.0; 4];
                        for i in 0..4 {
                            col[i] = c[0][i] * w0 + c[1][i] * w1 + c[2][i] * w2;
                        }
                        let u = v[0].uv.x * w0 + v[1].uv.x * w1 + v[2].uv.x * w2;
                        let vv = v[0].uv.y * w0 + v[1].uv.y * w1 + v[2].uv.y * w2;
                        modulate(col, texture.sample(u, vv))
                    }
                };
                if src[3] >= 254.5 {
                    for i in 0..4 {
                        dst[i] = src[i].clamp(0.0, 255.0) as u8;
                    }
                } else {
                    blend(dst, src);
                }
            }
            w0 += dw0_dx;
            w1 += dw1_dx;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_a_red_rectangle() {
        let ctx = egui::Context::default();
        let mut renderer = SoftwareRenderer::new(64, 64);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                Pos2::ZERO,
                egui::vec2(64.0, 64.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(input, |ctx| {
            ctx.layer_painter(egui::LayerId::background()).rect_filled(
                egui::Rect::from_min_max(Pos2::new(8.0, 8.0), Pos2::new(40.0, 40.0)),
                0.0,
                Color32::RED,
            );
        });
        renderer.update_textures(&out.textures_delta);
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        renderer.render(&prims, out.pixels_per_point);
        let px = |x: usize, y: usize| {
            let i = (y * 64 + x) * 4;
            [
                renderer.pixels[i],
                renderer.pixels[i + 1],
                renderer.pixels[i + 2],
            ]
        };
        assert_eq!(px(20, 20), [255, 0, 0]);
        assert_eq!(px(50, 50), [27, 27, 27]);
    }
}

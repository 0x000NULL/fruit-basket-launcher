//! Brand art: the fruit icons and the basket mark.
//!
//! Icons ship at 128 px and are drawn at 64 and 32, whole-number steps
//! down, so the pixel fruits stay on their 16 px grid (brand rule 3). The
//! mark is vector, drawn from its SVG at any size.

use std::collections::HashMap;

use basket_ui::canvas::{round_rect_path, segs_to_path};
use basket_ui::drawings::{parse, Drawing, Geometry};
use basket_ui::Canvas;
use tiny_skia::{IntSize, PathBuilder, Pixmap, Transform};

macro_rules! icons {
    ($($id:literal),*) => {
        &[$(($id,
            include_bytes!(concat!("../assets/icons/", $id, "-icon-128.png")).as_slice(),
            include_bytes!(concat!("../assets/icons/", $id, "-icon-night-128.png")).as_slice(),
        )),*]
    };
}

const ICONS: &[(&str, &[u8], &[u8])] =
    icons!("pomegranate", "strawberry", "fig", "starfruit", "mangosteen", "crabapple", "mulberry", "olive", "pear");

const MARK: &str = include_str!("../assets/fruit-basket-mark.svg");
const MARK_NIGHT: &str = include_str!("../assets/fruit-basket-mark-night.svg");
/// The mark's viewBox; the parser reads shapes, not the box.
const MARK_BOX: (f32, f32, f32, f32) = (8.0, 23.5, 104.0, 89.0);

/// Icon sizes the UI draws: 128 / 2 and 128 / 4.
pub const ICON_L: u32 = 64;
pub const ICON_S: u32 = 32;

pub struct Art {
    icons: HashMap<(String, bool, u32), Pixmap>,
    mark: Drawing,
    mark_night: Drawing,
}

impl Art {
    pub fn load() -> Art {
        let mut icons = HashMap::new();
        for (id, paper, night) in ICONS {
            for (night_flag, bytes) in [(false, paper), (true, night)] {
                let Some(full) = decode_png(bytes) else { continue };
                for size in [ICON_L, ICON_S] {
                    icons.insert((id.to_string(), night_flag, size), shrink(&full, 128 / size));
                }
            }
        }
        Art { icons, mark: parse(MARK), mark_night: parse(MARK_NIGHT) }
    }

    /// Draw a fruit's icon at `ICON_L` or `ICON_S`, faded to `alpha`
    /// (still-growing fruits are drawn at 45 %).
    pub fn icon(&self, cv: &mut Canvas, id: &str, night: bool, size: u32, x: f32, y: f32, alpha: f32) {
        let Some(pm) = self.icons.get(&(id.to_string(), night, size)) else { return };
        if alpha >= 1.0 {
            cv.draw_pixmap(pm, x.round(), y.round(), Transform::identity());
        } else {
            let mut faded = pm.clone();
            for px in faded.data_mut().chunks_exact_mut(4) {
                for c in px {
                    *c = (*c as f32 * alpha).round() as u8;
                }
            }
            cv.draw_pixmap(&faded, x.round(), y.round(), Transform::identity());
        }
    }

    /// Draw the basket mark `h` pixels tall with its top-left at (x, y).
    /// Returns its width.
    pub fn mark(&self, cv: &mut Canvas, night: bool, x: f32, y: f32, h: f32) -> f32 {
        let (bx, by, bw, bh) = MARK_BOX;
        let s = h / bh;
        let t = Transform::from_row(s, 0.0, 0.0, s, x - bx * s, y - by * s);
        draw_drawing(cv, if night { &self.mark_night } else { &self.mark }, t);
        bw * s
    }
}

/// `Canvas::drawing` has no scale, so draw the parsed shapes ourselves.
fn draw_drawing(cv: &mut Canvas, d: &Drawing, t: Transform) {
    for shape in &d.shapes {
        let path = match &shape.geom {
            Geometry::Rect { x, y, w, h, rx } => round_rect_path(*x, *y, *w, *h, [*rx; 4]),
            Geometry::Circle { cx, cy, r } => PathBuilder::from_circle(*cx, *cy, *r),
            Geometry::Path(segs) => segs_to_path(segs),
        };
        let Some(path) = path else { continue };
        if let Some(fill) = shape.fill {
            cv.fill_path(&path, fill, t);
        }
        if let Some(stroke) = shape.stroke {
            cv.stroke_path(&path, shape.stroke_width, stroke, shape.round_cap, t);
        }
    }
}

/// Decode an 8-bit RGBA PNG into a premultiplied pixmap.
pub(crate) fn decode_png(bytes: &[u8]) -> Option<Pixmap> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..(w * h * 4) as usize].to_vec(),
        png::ColorType::Rgb => buf[..(w * h * 3) as usize].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..(w * h * 2) as usize].chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buf[..(w * h) as usize].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    let premul: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| {
            let a = p[3] as u16;
            [(p[0] as u16 * a / 255) as u8, (p[1] as u16 * a / 255) as u8, (p[2] as u16 * a / 255) as u8, p[3]]
        })
        .collect();
    Pixmap::from_vec(premul, IntSize::from_wh(w, h)?)
}

/// Shrink by a whole factor, averaging each `f`×`f` block. On a pixel
/// fruit (16 px grid, 8 px per cell at 128) every block lies inside one
/// cell, so edges stay hard.
pub(crate) fn shrink(src: &Pixmap, f: u32) -> Pixmap {
    let (w, h) = (src.width() / f, src.height() / f);
    let mut out = Pixmap::new(w, h).expect("icon size");
    let s = src.data();
    let d = out.data_mut();
    let n = (f * f) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut acc = [0u32; 4];
            for dy in 0..f {
                for dx in 0..f {
                    let i = (((y * f + dy) * src.width() + x * f + dx) * 4) as usize;
                    for c in 0..4 {
                        acc[c] += s[i + c] as u32;
                    }
                }
            }
            let o = ((y * w + x) * 4) as usize;
            for c in 0..4 {
                d[o + c] = (acc[c] / n) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_decodes_at_both_sizes() {
        let art = Art::load();
        for (id, _, _) in ICONS {
            for night in [false, true] {
                assert_eq!(art.icons[&(id.to_string(), night, ICON_L)].width(), ICON_L, "{id}");
                assert_eq!(art.icons[&(id.to_string(), night, ICON_S)].width(), ICON_S, "{id}");
            }
        }
    }

    #[test]
    fn mark_parses() {
        let art = Art::load();
        assert!(art.mark.shapes.len() > 10);
        assert!(art.mark_night.shapes.len() > 10);
    }
}

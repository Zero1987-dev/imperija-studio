// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Where the faces are in a picture.
//!
//! YuNet, from the OpenCV model zoo: a quarter of a megabyte that reads a
//! frame and answers with a box per face. Small enough that a person waits
//! a second for it on first use rather than a minute, and quick enough on
//! a processor alone that reframing a clip does not need an accelerator.
//!
//! The model answers at three strides - one grid of cells every 8, 16 and
//! 32 pixels - and each cell of each grid offers one box. A cell says how
//! sure it is twice, once as a class and once as an object, and the two are
//! multiplied under a square root, which is how the model was trained to be
//! read. The box itself is the cell's own position nudged by two numbers
//! and sized by two more in log space. Boxes for the same face come out of
//! several cells, so what is left is thinned by [`nms`].
//!
//! Everything down to [`Detector`] is arithmetic over the numbers the model
//! gave, and is tested without one.
//!
//! Boxes leave here in source fractions, the language [`super::reframe`]
//! and the mask store already speak.

use super::reframe::Face;

/// The strides the model answers at.
pub const STRIDES: [usize; 3] = [8, 16, 32];

/// Below this a box is noise. YuNet is confident about a face that is
/// actually there; the doubtful ones are mostly patterned backgrounds.
pub const SCORE: f64 = 0.6;

/// How much two boxes may overlap before the weaker is dropped as a second
/// answer for the same face.
pub const IOU: f64 = 0.3;

/// The model reads a picture this wide, and no other width.
///
/// Not a choice: this build of YuNet is exported with its input shape
/// fixed, and the runtime refuses anything else outright - "Got: 320,
/// Expected: 640". A smaller input would be quicker and quite enough for
/// faces the size a podcast frames them, but the model has to be re-
/// exported to accept one, and a working detector beats a faster refusal.
pub const INPUT_W: usize = 640;

/// And this tall, for the same reason. Square, so the picture is
/// letterboxed into it - see [`fit`] - whatever shape it came in.
pub const INPUT_H: usize = 640;

/// One stride's three answers, as the model laid them out.
pub struct Level<'a> {
    /// How far apart this grid's cells are, in model pixels.
    pub stride: usize,
    /// Class score per cell.
    pub cls: &'a [f32],
    /// Objectness per cell.
    pub obj: &'a [f32],
    /// Four numbers per cell: the nudge to its middle, then the size.
    pub bbox: &'a [f32],
}

/// How a picture sits inside the model's input: scaled to fit, centred,
/// with bars where the aspects differ.
///
/// Letterboxing and not stretching, because a stretched face is a shape
/// the model was not trained on and it finds fewer of them. The numbers
/// here undo it: a box in model pixels goes back to source fractions.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Fit {
    /// Model pixels per source pixel.
    pub scale: f64,
    /// Model pixels of bar down the left.
    pub pad_x: f64,
    /// Model pixels of bar along the top.
    pub pad_y: f64,
    /// The source's size, for turning pixels back into fractions.
    pub source: (f64, f64),
}

/// How a source of this size fits the model's input.
pub fn fit(width: f64, height: f64) -> Fit {
    if width <= 0.0 || height <= 0.0 {
        return Fit {
            scale: 1.0,
            pad_x: 0.0,
            pad_y: 0.0,
            source: (1.0, 1.0),
        };
    }
    let scale = (INPUT_W as f64 / width).min(INPUT_H as f64 / height);
    Fit {
        scale,
        pad_x: (INPUT_W as f64 - width * scale) / 2.0,
        pad_y: (INPUT_H as f64 - height * scale) / 2.0,
        source: (width, height),
    }
}

impl Fit {
    /// A box in model pixels, back in source fractions. Clamped to the
    /// picture: a face at the edge answers a little past it.
    pub fn undo(&self, x: f64, y: f64, w: f64, h: f64) -> (f64, f64, f64, f64) {
        let (sw, sh) = self.source;
        let to_x = |v: f64| ((v - self.pad_x) / self.scale / sw).clamp(0.0, 1.0);
        let to_y = |v: f64| ((v - self.pad_y) / self.scale / sh).clamp(0.0, 1.0);
        let (x0, y0) = (to_x(x), to_y(y));
        let (x1, y1) = (to_x(x + w), to_y(y + h));
        (x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
    }
}

/// The faces one stride's grid offers, above [`SCORE`].
///
/// A cell at column `c`, row `r` describes a box whose middle is
/// `(c + bbox[0]) * stride` across and `(r + bbox[1]) * stride` down, and
/// whose size is `exp(bbox[2..4]) * stride`. The sizes are in log space so
/// that one small network can answer for faces an order of magnitude apart.
pub fn decode(level: &Level, at: &Fit) -> Vec<Face> {
    let cols = INPUT_W / level.stride;
    let rows = INPUT_H / level.stride;
    let cells = cols * rows;
    if level.cls.len() < cells || level.obj.len() < cells || level.bbox.len() < cells * 4 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 0..cells {
        let score = (f64::from(level.cls[i]).clamp(0.0, 1.0)
            * f64::from(level.obj[i]).clamp(0.0, 1.0))
        .sqrt();
        if score < SCORE {
            continue;
        }
        let (col, row) = (i % cols, i / cols);
        let b = &level.bbox[i * 4..i * 4 + 4];
        let s = level.stride as f64;
        let w = f64::from(b[2]).exp() * s;
        let h = f64::from(b[3]).exp() * s;
        let cx = (col as f64 + f64::from(b[0])) * s;
        let cy = (row as f64 + f64::from(b[1])) * s;
        let (x, y, w, h) = at.undo(cx - w / 2.0, cy - h / 2.0, w, h);
        if w > 0.0 && h > 0.0 {
            out.push(Face { x, y, w, h, score });
        }
    }
    out
}

/// How much two boxes overlap, as a fraction of the area they cover
/// together. Zero when they do not touch, one when they are the same box.
pub fn iou(a: &Face, b: &Face) -> f64 {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    let overlap = (x1 - x0).max(0.0) * (y1 - y0).max(0.0);
    let union = a.area() + b.area() - overlap;
    if union <= 0.0 { 0.0 } else { overlap / union }
}

/// One box per face: the surest kept, and anything overlapping it by more
/// than [`IOU`] dropped as another answer for the same face.
pub fn nms(mut faces: Vec<Face>, threshold: f64) -> Vec<Face> {
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Face> = Vec::new();
    for face in faces {
        if !kept.iter().any(|k| iou(k, &face) > threshold) {
            kept.push(face);
        }
    }
    kept
}

/// The picture as the model reads it: three planes of [`INPUT_W`] by
/// [`INPUT_H`], blue then green then red, 0 to 255, the source letterboxed
/// into the middle and the bars left black.
///
/// The size has to be exactly the model's own. It is fixed in this export,
/// and feeding it anything else is refused by the runtime before a single
/// frame is read - which is how a detector that never ran got as far as a
/// person clicking the button.
///
/// Blue first because that is the order the model was trained in, and
/// unscaled because YuNet takes raw values rather than the 0 to 1 most
/// other models want. Sampling is nearest-neighbour: the input is small,
/// the boxes are coarse, and an average would cost more than it buys.
#[cfg(feature = "infer")]
pub fn planes(frame: &concat_core::Frame) -> (Vec<f32>, Fit) {
    use concat_core::frame::BYTES_PER_PIXEL;
    let (fw, fh) = (frame.width() as usize, frame.height() as usize);
    let at = fit(fw as f64, fh as f64);
    let mut data = vec![0f32; 3 * INPUT_W * INPUT_H];
    if fw == 0 || fh == 0 {
        return (data, at);
    }
    let pixels = frame.pixels();
    for y in 0..INPUT_H {
        let sy = (y as f64 - at.pad_y) / at.scale;
        if sy < 0.0 || sy >= fh as f64 {
            continue;
        }
        let sy = sy as usize;
        for x in 0..INPUT_W {
            let sx = (x as f64 - at.pad_x) / at.scale;
            if sx < 0.0 || sx >= fw as f64 {
                continue;
            }
            let src = (sy * fw + sx as usize) * BYTES_PER_PIXEL;
            // Blue, green, red - the model's order, not the frame's.
            for (plane, channel) in [2usize, 1, 0].into_iter().enumerate() {
                data[(plane * INPUT_H + y) * INPUT_W + x] = f32::from(pixels[src + channel]);
            }
        }
    }
    (data, at)
}

/// The model, loaded once and asked for a frame at a time.
#[cfg(feature = "infer")]
pub struct Detector {
    model: super::runtime::Model,
}

#[cfg(feature = "infer")]
impl Detector {
    /// Loads the model from the file [`super::models`] fetched.
    pub fn from_file(path: &std::path::Path) -> Result<Detector, String> {
        Ok(Detector {
            model: super::runtime::Model::from_file(path)?,
        })
    }

    /// Every face in a frame, one box each, in source fractions.
    pub fn faces(&mut self, frame: &concat_core::Frame) -> Result<Vec<Face>, String> {
        use super::runtime::Input;
        let (data, at) = planes(frame);
        let input = Input {
            name: "input",
            dims: vec![1, 3, INPUT_H, INPUT_W],
            data: data.into(),
        };
        let names: Vec<String> = STRIDES
            .iter()
            .flat_map(|s| [format!("cls_{s}"), format!("obj_{s}"), format!("bbox_{s}")])
            .collect();
        let wanted: Vec<&str> = names.iter().map(String::as_str).collect();
        let out = self.model.run(vec![input], &wanted)?;
        if out.len() != wanted.len() {
            return Err(format!(
                "the detector answered {} of {} outputs",
                out.len(),
                wanted.len()
            ));
        }
        let mut faces = Vec::new();
        for (i, stride) in STRIDES.iter().enumerate() {
            faces.extend(decode(
                &Level {
                    stride: *stride,
                    cls: &out[i * 3].data,
                    obj: &out[i * 3 + 1].data,
                    bbox: &out[i * 3 + 2].data,
                },
                &at,
            ));
        }
        Ok(nms(faces, IOU))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grids_are_the_size_the_model_says_they_are() {
        // Read out of the model file itself: input [1, 3, 640, 640], and
        // cls_8 / cls_16 / cls_32 of 6400, 1600 and 400 cells. The input
        // size is fixed in this export, so a mismatch is not a worse
        // answer - the runtime refuses the frame outright, which is how a
        // detector that had never run reached a person's finger.
        assert_eq!((INPUT_W, INPUT_H), (640, 640));
        for (stride, cells) in [(8, 6400), (16, 1600), (32, 400)] {
            let cols = INPUT_W / stride;
            let rows = INPUT_H / stride;
            assert_eq!(cols * rows, cells, "stride {stride}");
        }
        assert_eq!(STRIDES, [8, 16, 32]);
    }

    #[test]
    fn a_wide_picture_gets_bars_above_and_below() {
        let at = fit(1920.0, 1080.0);
        // 320 / 1920 is the tighter of the two, so it sets the scale.
        assert!((at.scale - INPUT_W as f64 / 1920.0).abs() < 1e-12);
        assert!(at.pad_x.abs() < 1e-9, "no bars at the sides: {}", at.pad_x);
        assert!(at.pad_y > 0.0, "bars above and below: {}", at.pad_y);
    }

    #[test]
    fn undoing_the_fit_lands_back_where_it_started() {
        let at = fit(1920.0, 1080.0);
        // A box over the middle of the source, taken into model pixels.
        let (sx, sy, sw, sh) = (960.0, 540.0, 192.0, 108.0);
        let (x, y, w, h) = at.undo(
            sx * at.scale + at.pad_x,
            sy * at.scale + at.pad_y,
            sw * at.scale,
            sh * at.scale,
        );
        assert!((x - 0.5).abs() < 1e-6, "{x}");
        assert!((y - 0.5).abs() < 1e-6, "{y}");
        assert!((w - 0.1).abs() < 1e-6, "{w}");
        assert!((h - 0.1).abs() < 1e-6, "{h}");
    }

    /// One grid with a single confident cell, and everything else silent.
    fn level(stride: usize, cell: usize, b: [f32; 4]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let cells = (INPUT_W / stride) * (INPUT_H / stride);
        let mut cls = vec![0.0; cells];
        let mut obj = vec![0.0; cells];
        let mut bbox = vec![0.0; cells * 4];
        cls[cell] = 1.0;
        obj[cell] = 1.0;
        bbox[cell * 4..cell * 4 + 4].copy_from_slice(&b);
        (cls, obj, bbox)
    }

    #[test]
    fn a_confident_cell_becomes_a_box_where_that_cell_is() {
        let stride = 32;
        let cols = INPUT_W / stride;
        let (row, col) = (3usize, 5usize);
        // No nudge, and a box two cells across.
        let (cls, obj, bbox) = level(stride, row * cols + col, [0.0, 0.0, 2f32.ln(), 2f32.ln()]);
        let at = fit(INPUT_W as f64, INPUT_H as f64);
        let faces = decode(
            &Level {
                stride,
                cls: &cls,
                obj: &obj,
                bbox: &bbox,
            },
            &at,
        );
        assert_eq!(faces.len(), 1, "{faces:?}");
        let f = faces[0];
        let (u, v) = f.centre();
        assert!(
            (u - (col as f64 * 32.0) / INPUT_W as f64).abs() < 1e-6,
            "{u}"
        );
        assert!(
            (v - (row as f64 * 32.0) / INPUT_H as f64).abs() < 1e-6,
            "{v}"
        );
        assert!((f.w - 64.0 / INPUT_W as f64).abs() < 1e-6, "{}", f.w);
    }

    #[test]
    fn a_doubtful_cell_is_not_a_face() {
        let stride = 32;
        let (mut cls, obj, bbox) = level(stride, 10, [0.0, 0.0, 0.0, 0.0]);
        // sqrt(0.2 * 1.0) is under the threshold.
        cls[10] = 0.2;
        let at = fit(INPUT_W as f64, INPUT_H as f64);
        assert!(
            decode(
                &Level {
                    stride,
                    cls: &cls,
                    obj: &obj,
                    bbox: &bbox
                },
                &at
            )
            .is_empty()
        );
    }

    #[test]
    fn an_answer_that_is_the_wrong_size_is_refused_rather_than_read_past() {
        let at = fit(320.0, 240.0);
        let short = vec![1.0; 4];
        let faces = decode(
            &Level {
                stride: 8,
                cls: &short,
                obj: &short,
                bbox: &short,
            },
            &at,
        );
        assert!(faces.is_empty());
    }

    fn box_at(x: f64, y: f64, w: f64, score: f64) -> Face {
        Face {
            x,
            y,
            w,
            h: w,
            score,
        }
    }

    #[test]
    fn overlapping_answers_for_one_face_come_out_as_one() {
        let faces = vec![
            box_at(0.10, 0.10, 0.20, 0.80),
            box_at(0.11, 0.11, 0.20, 0.95),
            box_at(0.60, 0.10, 0.20, 0.90),
        ];
        let kept = nms(faces, IOU);
        assert_eq!(kept.len(), 2, "{kept:?}");
        // The surest of the overlapping pair survives, and it comes first.
        assert!((kept[0].score - 0.95).abs() < 1e-9);
    }

    #[test]
    fn two_people_side_by_side_both_survive() {
        let kept = nms(
            vec![box_at(0.05, 0.4, 0.15, 0.9), box_at(0.80, 0.4, 0.15, 0.9)],
            IOU,
        );
        assert_eq!(kept.len(), 2);
    }

    #[test]
    fn a_box_with_itself_overlaps_entirely_and_with_a_stranger_not_at_all() {
        let a = box_at(0.1, 0.1, 0.2, 0.9);
        assert!((iou(&a, &a) - 1.0).abs() < 1e-9);
        assert!(iou(&a, &box_at(0.7, 0.7, 0.2, 0.9)).abs() < 1e-9);
    }
}

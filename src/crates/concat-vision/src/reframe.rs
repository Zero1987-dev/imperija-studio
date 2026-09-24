// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 TikTok Imperija
//! Reframing a wide shot to a tall one, following whoever is talking.
//!
//! A podcast is shot wide and posted tall. Done by hand that is a keyframe
//! every time someone leans, and it is the reason the clip never gets cut.
//! Done here it is one pass: a face detector says where the subject is in
//! each sampled frame, and this module turns that into the `scale`,
//! `offsetX` and `offsetY` the clip already understands.
//!
//! Those three and not a new effect on purpose. The renderer, the exporter
//! and the Keyframes tab all read them today, so a reframe is an edit the
//! user can see, drag and undo rather than a black box. If the tracker put
//! the camera somewhere silly, the fix is moving a key.
//!
//! Everything here is geometry and arithmetic, tested without a model. The
//! detector is [`super::face`], behind the `infer` feature.
//!
//! **Fractions, not pixels.** A face arrives as fractions of the source
//! picture - `(0, 0)` its top-left, `(1, 1)` its bottom-right - which is
//! the same language masks and strokes are stored in, so a crop or a change
//! of export size changes nothing here.

/// Where a face is in the source picture, in source fractions.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Face {
    /// Left edge, `0..1`.
    pub x: f64,
    /// Top edge, `0..1`.
    pub y: f64,
    /// Width as a fraction of the source's width.
    pub w: f64,
    /// Height as a fraction of the source's height.
    pub h: f64,
    /// How sure the detector is, `0..1`.
    pub score: f64,
}

impl Face {
    /// The middle of the face, in source fractions.
    pub fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }

    /// How much of the picture it covers. The subject is usually the
    /// largest face; a face in the background is smaller by the same
    /// amount it is unimportant.
    pub fn area(&self) -> f64 {
        (self.w * self.h).max(0.0)
    }
}

/// Where the camera is, in the clip's own units.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Shot {
    /// `Clip::scale`: multiplier over the fitted size, 1 filling the frame.
    pub scale: f64,
    /// `Clip::offset_x`: from centred, as a fraction of frame width.
    pub offset_x: f64,
    /// `Clip::offset_y`: the same vertically; positive moves down.
    pub offset_y: f64,
}

/// The scale at which a source of one aspect covers a frame of another,
/// leaving no bars.
///
/// At `scale` 1 the source is *fitted*: the whole of it is inside the frame
/// and the rest is empty. Filling instead means growing it until its short
/// side reaches the frame's, which is the ratio of the two aspects - 3.16
/// for 16:9 into 9:16, because a tall slice of a wide picture is under a
/// third of its width.
pub fn cover_scale(source_aspect: f64, frame_aspect: f64) -> f64 {
    if source_aspect <= 0.0 || frame_aspect <= 0.0 {
        return 1.0;
    }
    (source_aspect / frame_aspect).max(frame_aspect / source_aspect)
}

/// How far the camera may travel before it shows past the picture's edge,
/// as a fraction of the frame, horizontally and vertically.
///
/// At scale `s` the source is `s` frame-widths across, so half of `s - 1`
/// hangs off each side and that is exactly how far it may move. Vertically
/// the source is `s * frame_aspect / source_aspect` frame-heights tall, and
/// a wide source filled into a tall frame has nothing to spare there: the
/// limit is zero and the camera only pans sideways, which is what a podcast
/// wants anyway.
pub fn travel(scale: f64, source_aspect: f64, frame_aspect: f64) -> (f64, f64) {
    let across = ((scale - 1.0) / 2.0).max(0.0);
    let down = if source_aspect > 0.0 {
        ((scale * frame_aspect / source_aspect - 1.0) / 2.0).max(0.0)
    } else {
        0.0
    };
    (across, down)
}

/// The camera that puts a point of the source under the middle of the frame.
///
/// A point at source fraction `u` sits `(u - 0.5)` of the *displayed* width
/// from its middle, and the displayed width is `scale` frames across, so
/// bringing it to the middle is an offset of `-(u - 0.5) * scale`. The
/// vertical is the same with the displayed height, which is shorter by the
/// ratio of the aspects. Both are then held inside [`travel`], so the frame
/// never shows past the edge of the picture.
pub fn shot_for(u: f64, v: f64, scale: f64, source_aspect: f64, frame_aspect: f64) -> Shot {
    let height_in_frames = if source_aspect > 0.0 {
        scale * frame_aspect / source_aspect
    } else {
        scale
    };
    let (max_x, max_y) = travel(scale, source_aspect, frame_aspect);
    Shot {
        scale,
        offset_x: (-(u - 0.5) * scale).clamp(-max_x, max_x),
        offset_y: (-(v - 0.5) * height_in_frames).clamp(-max_y, max_y),
    }
}

/// Which face the camera follows.
///
/// The largest, and among faces of a similar size the one nearest the
/// middle. Size alone flickers between two people sitting the same distance
/// away - the detector's boxes wobble by a percent or two from frame to
/// frame - so anything within a tenth of the largest counts as the same
/// size and the tie is broken by position, which does not wobble.
pub fn pick_subject(faces: &[Face]) -> Option<Face> {
    let largest = faces.iter().map(Face::area).fold(0.0_f64, f64::max);
    if largest <= 0.0 {
        return None;
    }
    faces
        .iter()
        .filter(|f| f.area() >= largest * 0.9)
        .min_by(|a, b| {
            let d = |f: &Face| {
                let (u, v) = f.centre();
                (u - 0.5).powi(2) + (v - 0.5).powi(2)
            };
            d(a).total_cmp(&d(b))
        })
        .copied()
}

/// How far the subject may drift before the camera answers, in source
/// fractions.
///
/// A head moves a little all the time. Following every twitch reads as a
/// hand-held camera in a room where there is none, so inside this the
/// camera holds still.
pub const DEADZONE: f64 = 0.04;

/// How much of the way the camera closes on the subject each sampled frame.
///
/// Low enough that a turn of the head is a glide and not a snap, high
/// enough that the camera has arrived before the sentence ends.
pub const EASING: f64 = 0.12;

/// A jump this far or further is a cut between speakers, not a movement.
///
/// Gliding across a third of the picture takes seconds and looks like the
/// camera lost someone. When the subject changes by this much the camera is
/// simply there, the way a vision mixer would have cut.
pub const CUT: f64 = 0.33;

/// How near the camera has to get before it stops following again.
///
/// The deadzone decides when the camera *sets out*, not when it gives up.
/// Without this pair the camera would stop the moment it came inside
/// [`DEADZONE`] and sit that far off-centre for the rest of the clip;
/// with it a real move is seen through to the end, and only then does the
/// camera go back to ignoring small ones.
pub const ARRIVED: f64 = DEADZONE / 4.0;

/// The camera's path over a clip, from where the subject was in each
/// sampled frame.
///
/// The camera is either holding or following. Holding, it ignores anything
/// inside [`DEADZONE`] and sets out for anything past it. Following, it
/// closes [`EASING`] of the distance each sample until it is within
/// [`ARRIVED`], and then holds again. A jump of [`CUT`] or more is not a
/// move at all - it is the other person talking - so the camera is simply
/// there.
///
/// A frame nobody was found in keeps the camera where it was: a subject who
/// turns away or is briefly hidden should not send it back to the middle.
pub fn follow(subjects: &[Option<(f64, f64)>]) -> Vec<(f64, f64)> {
    let start = subjects
        .iter()
        .flatten()
        .next()
        .copied()
        .unwrap_or((0.5, 0.5));
    let mut camera = start;
    let mut following = false;
    let mut path = Vec::with_capacity(subjects.len());
    for seen in subjects {
        if let Some((u, v)) = *seen {
            let (dx, dy) = (u - camera.0, v - camera.1);
            let distance = (dx * dx + dy * dy).sqrt();
            if distance >= CUT {
                camera = (u, v);
                following = false;
            } else {
                if distance > DEADZONE {
                    following = true;
                } else if distance <= ARRIVED {
                    following = false;
                }
                if following {
                    camera = (camera.0 + dx * EASING, camera.1 + dy * EASING);
                }
            }
        }
        path.push(camera);
    }
    path
}

/// The path as keys, with the ones that say nothing left out.
///
/// A key per sampled frame is thousands of keys on a clip nobody can then
/// edit. A point is kept when the straight line between the last kept key
/// and the next one would miss it by more than `tolerance` - the
/// Ramer-Douglas-Peucker rule - so a still camera is two keys and a pan is
/// as many as its shape needs. The first and last are always kept.
pub fn reduce(path: &[(f64, f64)], tolerance: f64) -> Vec<usize> {
    if path.len() <= 2 {
        return (0..path.len()).collect();
    }
    let mut keep = vec![false; path.len()];
    keep[0] = true;
    keep[path.len() - 1] = true;
    simplify(path, 0, path.len() - 1, tolerance, &mut keep);
    (0..path.len()).filter(|i| keep[*i]).collect()
}

/// The recursive half of [`reduce`]: the point furthest from the chord
/// between two kept ones, kept in turn when it is further than the
/// tolerance, and the two halves either side done the same way.
fn simplify(path: &[(f64, f64)], first: usize, last: usize, tolerance: f64, keep: &mut [bool]) {
    if last <= first + 1 {
        return;
    }
    let (ax, ay) = path[first];
    let (bx, by) = path[last];
    let (dx, dy) = (bx - ax, by - ay);
    let length = (dx * dx + dy * dy).sqrt();
    let mut worst = (0.0_f64, first);
    for (i, &(px, py)) in path.iter().enumerate().take(last).skip(first + 1) {
        // With no chord to speak of, distance from the point is the measure.
        let d = if length <= f64::EPSILON {
            ((px - ax).powi(2) + (py - ay).powi(2)).sqrt()
        } else {
            ((bx - ax) * (ay - py) - (ax - px) * (by - ay)).abs() / length
        };
        if d > worst.0 {
            worst = (d, i);
        }
    }
    if worst.0 > tolerance {
        simplify(path, first, worst.1, tolerance, keep);
        keep[worst.1] = true;
        simplify(path, worst.1, last, tolerance, keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: f64 = 16.0 / 9.0;
    const TALL: f64 = 9.0 / 16.0;

    #[test]
    fn filling_a_tall_frame_from_a_wide_one_is_the_ratio_of_the_aspects() {
        // A 9:16 slice of a 16:9 picture is 31.6% of its width.
        let s = cover_scale(WIDE, TALL);
        assert!((s - (WIDE / TALL)).abs() < 1e-9);
        assert!((s - 3.160_493).abs() < 1e-5, "{s}");
        // The same source and frame needs no growing at all.
        assert!((cover_scale(WIDE, WIDE) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_filled_tall_frame_pans_sideways_and_not_up() {
        let s = cover_scale(WIDE, TALL);
        let (across, down) = travel(s, WIDE, TALL);
        assert!(across > 1.0, "sideways room: {across}");
        assert!(down.abs() < 1e-9, "no vertical room once filled: {down}");
    }

    #[test]
    fn the_camera_puts_the_subject_in_the_middle() {
        let s = cover_scale(WIDE, TALL);
        // Someone a quarter of the way in from the left.
        let shot = shot_for(0.25, 0.5, s, WIDE, TALL);
        // Offset is measured in frame widths: a quarter of the source is a
        // quarter of `scale` frames.
        assert!((shot.offset_x - 0.25 * s).abs() < 1e-9, "{:?}", shot);
        assert!(shot.offset_y.abs() < 1e-9);
    }

    #[test]
    fn the_camera_never_shows_past_the_edge() {
        let s = cover_scale(WIDE, TALL);
        let (across, _) = travel(s, WIDE, TALL);
        for u in [0.0, 0.02, 0.98, 1.0] {
            let shot = shot_for(u, 0.5, s, WIDE, TALL);
            assert!(shot.offset_x.abs() <= across + 1e-9, "u={u} {:?}", shot);
        }
    }

    fn face(x: f64, y: f64, w: f64) -> Face {
        Face {
            x,
            y,
            w,
            h: w,
            score: 0.9,
        }
    }

    #[test]
    fn the_subject_is_the_largest_face() {
        let faces = [face(0.1, 0.4, 0.05), face(0.6, 0.4, 0.20)];
        let picked = pick_subject(&faces).expect("a face");
        assert!((picked.w - 0.20).abs() < 1e-9);
    }

    #[test]
    fn two_faces_of_a_size_do_not_flicker_the_camera_between_them() {
        // Same size within the detector's wobble; the middle one wins, and
        // keeps winning when the other one's box grows a percent.
        let near = face(0.45, 0.45, 0.100);
        let far = face(0.05, 0.45, 0.098);
        assert_eq!(pick_subject(&[near, far]).unwrap().x, near.x);
        let far_grown = face(0.05, 0.45, 0.102);
        assert_eq!(pick_subject(&[near, far_grown]).unwrap().x, near.x);
    }

    #[test]
    fn no_face_is_no_subject() {
        assert!(pick_subject(&[]).is_none());
    }

    #[test]
    fn a_still_head_holds_the_camera_still() {
        // Drift well inside the deadzone, every sample.
        let seen: Vec<_> = (0..40)
            .map(|i| Some((0.4 + (i % 2) as f64 * 0.01, 0.5)))
            .collect();
        let path = follow(&seen);
        for p in &path {
            assert!((p.0 - 0.4).abs() < 1e-9, "the camera moved to {p:?}");
        }
    }

    #[test]
    fn a_walk_is_followed_but_never_snapped_to() {
        let seen: Vec<_> = (0..60).map(|_| Some((0.8, 0.5))).collect();
        let path = follow(&seen);
        // It sets out from the subject's first position, so a single
        // sample is already there; give it a start away from the target.
        let mut seen2 = vec![Some((0.2, 0.5))];
        seen2.extend(std::iter::repeat_n(Some((0.5, 0.5)), 60));
        let path2 = follow(&seen2);
        assert!((path[0].0 - 0.8).abs() < 1e-9);
        // Under the cut distance, so it glides: not there on the first step.
        assert!(path2[1].0 > 0.2 && path2[1].0 < 0.5, "{:?}", path2[1]);
        // And has arrived by the end.
        assert!((path2.last().unwrap().0 - 0.5).abs() < 0.01);
    }

    #[test]
    fn a_change_of_speaker_is_a_cut_and_not_a_glide() {
        let mut seen = vec![Some((0.15, 0.5)); 5];
        seen.extend(std::iter::repeat_n(Some((0.85, 0.5)), 5));
        let path = follow(&seen);
        assert!((path[4].0 - 0.15).abs() < 1e-9);
        assert!((path[5].0 - 0.85).abs() < 1e-9, "should cut: {:?}", path[5]);
    }

    #[test]
    fn a_frame_with_nobody_in_it_leaves_the_camera_where_it_was() {
        let seen = vec![Some((0.8, 0.5)), None, None, Some((0.8, 0.5))];
        let path = follow(&seen);
        assert_eq!(path[1], path[0]);
        assert_eq!(path[2], path[0]);
    }

    #[test]
    fn a_still_camera_reduces_to_its_two_ends() {
        let path = vec![(0.5, 0.5); 300];
        assert_eq!(reduce(&path, 0.002), vec![0, 299]);
    }

    #[test]
    fn a_pan_keeps_the_points_that_carry_its_shape() {
        // Straight travel needs no middle keys.
        let straight: Vec<_> = (0..=100).map(|i| (i as f64 / 100.0, 0.5)).collect();
        assert_eq!(reduce(&straight, 0.002), vec![0, 100]);
        // A corner has to survive.
        let mut bent: Vec<(f64, f64)> = (0..=50).map(|i| (i as f64 / 100.0, 0.5)).collect();
        bent.extend((51..=100).map(|i| (0.5, 0.5 + (i - 50) as f64 / 100.0)));
        let kept = reduce(&bent, 0.002);
        assert!(kept.len() >= 3 && kept.len() < 20, "kept {kept:?}");
        assert!(kept.contains(&0) && kept.contains(&100));
    }

    #[test]
    fn two_points_or_fewer_are_all_kept() {
        assert_eq!(reduce(&[], 0.01), Vec::<usize>::new());
        assert_eq!(reduce(&[(0.5, 0.5)], 0.01), vec![0]);
        assert_eq!(reduce(&[(0.1, 0.1), (0.9, 0.9)], 0.01), vec![0, 1]);
    }
}

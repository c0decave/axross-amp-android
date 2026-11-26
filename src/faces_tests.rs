//! Every face at a phone's sizes (the audit's test 4): a sliver of split
//! screen, a touch target, a phone upright and on its side at 3 pixels a
//! point, a tablet at 2 - drawn for a few frames each, with music and in
//! silence, on the CPU (the faces' GPU path is the desktop's and is checked
//! where it runs). None may panic; the flash guard draws over all of them.

use crate::amp::analyzer::Analyzer;
use crate::amp::state::VisualOptions;
use crate::amp::viz::{self, VizState};
use eframe::egui;

/// (width, height) in points, pixels a point.
const SIZES: [(f32, f32, f32); 5] = [
    (1.0, 1.0, 1.0),
    (48.0, 48.0, 1.0),
    (360.0, 780.0, 3.0),
    (780.0, 360.0, 3.0),
    (1280.0, 800.0, 2.0),
];

/// A tone and its harmonics in both channels, `frames` long from `start`.
fn music(start: u64, frames: usize, rate: f32) -> Vec<f32> {
    (0..frames)
        .flat_map(|i| {
            let t = (start + i as u64) as f32 / rate;
            let v = 0.4 * (t * 440.0 * std::f32::consts::TAU).sin()
                + 0.2 * (t * 1320.0 * std::f32::consts::TAU).sin();
            [v, v * 0.8]
        })
        .collect()
}

#[test]
fn every_face_draws_at_a_phones_sizes() {
    let ctx = egui::Context::default();
    let options = VisualOptions::default();
    let scheme = viz::scheme("classic", &Default::default());
    let faces: Vec<&'static str> = viz::FACES
        .iter()
        .filter(|f| f.group != viz::Group::Off)
        .map(|f| f.key)
        .collect();
    assert!(faces.len() > 60, "only {} faces", faces.len());
    let rate = 48_000u32;
    for (width, height, ppp) in SIZES {
        for face in &faces {
            let mut state = VizState::default();
            state.set_gpu(false);
            let mut analyzer = Analyzer::new();
            let mut written = 0u64;
            for frame in 0..4 {
                // (the first frames with music, the last in silence)
                let tap = if frame < 3 {
                    music(written, 800, rate as f32)
                } else {
                    vec![0.0; 1600]
                };
                written += 800;
                analyzer.update(
                    &tap,
                    written,
                    rate,
                    &options,
                    1.0 / 60.0,
                    frame < 3,
                    viz::needs_now(face, &state),
                );
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, height),
                    )),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                };
                ctx.set_pixels_per_point(ppp);
                let mut out = ctx.run_ui(input, |ui| {
                    let rect = ui.max_rect();
                    let f = viz::Frame {
                        painter: ui.painter(),
                        rect,
                        a: &analyzer,
                        s: &scheme,
                        dt: 1.0 / 60.0,
                        title: "A title of some length - and its artist",
                        overview: None,
                        position: frame as f32,
                        duration: 200.0,
                        hover: None,
                        opacity: 1.0,
                        second: false,
                        loudness_target: -23.0,
                        cover: None,
                    };
                    viz::draw(&f, face, &mut state);
                });
                // (no renderer here to take the textures)
                out.textures_delta.clear();
            }
        }
    }
}

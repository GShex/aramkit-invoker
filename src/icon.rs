/// Size of the generated application/tray icon.
pub const SIZE: u32 = 64;

const WAITING_COLOR: [u8; 3] = [91, 100, 120];
const CONNECTED_COLOR: [u8; 3] = [124, 92, 255];

/// Straight-alpha RGBA pixels for the tray icon.
pub fn rgba(connected: bool) -> Vec<u8> {
    render(if connected {
        CONNECTED_COLOR
    } else {
        WAITING_COLOR
    })
}

/// Window icon data for eframe.
pub fn egui_icon(connected: bool) -> egui::IconData {
    egui::IconData {
        rgba: rgba(connected),
        width: SIZE,
        height: SIZE,
    }
}

fn render(accent: [u8; 3]) -> Vec<u8> {
    const SAMPLES: u32 = 4;
    const SAMPLES_TOTAL: f32 = (SAMPLES * SAMPLES) as f32;

    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut circle_coverage = 0u32;
            let mut gem_coverage = 0u32;

            for sample_y in 0..SAMPLES {
                for sample_x in 0..SAMPLES {
                    let px = x as f32 + (sample_x as f32 + 0.5) / SAMPLES as f32;
                    let py = y as f32 + (sample_y as f32 + 0.5) / SAMPLES as f32;
                    let dx = px - SIZE as f32 / 2.0;
                    let dy = py - SIZE as f32 / 2.0;

                    if dx * dx + dy * dy <= 27.0_f32.powi(2) {
                        circle_coverage += 1;
                    }
                    if dx.abs() + dy.abs() <= 13.0 {
                        gem_coverage += 1;
                    }
                }
            }

            if circle_coverage == 0 {
                continue;
            }

            let circle_alpha = circle_coverage as f32 / SAMPLES_TOTAL;
            let gem_alpha = (gem_coverage as f32 / SAMPLES_TOTAL) * circle_alpha;

            let index = ((y * SIZE + x) * 4) as usize;
            for channel in 0..3 {
                let base = accent[channel] as f32;
                let white = 255.0;
                pixels[index + channel] =
                    (base * (1.0 - gem_alpha) + white * gem_alpha).round() as u8;
            }
            pixels[index + 3] = (circle_alpha * 255.0).round() as u8;
        }
    }

    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_expected_rgba_length() {
        assert_eq!(rgba(true).len(), (SIZE * SIZE * 4) as usize);
    }

    #[test]
    fn icon_has_visible_pixels_and_transparent_corners() {
        let pixels = rgba(true);
        assert!(pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] > 0));
        assert_eq!(pixels[3], 0);
    }

    #[test]
    fn connection_state_changes_icon_color() {
        assert_ne!(rgba(true), rgba(false));
    }
}

//! Renders the real UI with the software renderer, as the overlay and the
//! desktop window would show it.
//!
//! `PSSTATES_RENDER_DIR=some/dir cargo test -p psstates-vr --release --test render_main_ui -- --nocapture`
//! writes the images as PNGs and prints frame timings.

use std::time::{Duration, Instant};

use psstates_core::Store;
use psstates_service::Service;
use psstates_ui::MainUi;
use psstates_vr::SoftwareRenderer;

const BACKGROUND: [u8; 3] = [27, 27, 27];

struct Setup {
    _rt: tokio::runtime::Runtime,
    _dir: tempfile::TempDir,
    service: Service,
}

fn setup() -> Setup {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    // No mDNS: a VRChat running on this machine must not change the result.
    let service = Service::start_with(rt.handle().clone(), store, false).unwrap();
    service
        .store()
        .update_avatar("avtr_demo", |a| {
            for name in [
                "Casual",
                "Winter outfit with a very long name that must be truncated nicely",
                "Dance floor",
                "Gym",
                "Beach",
            ] {
                a.add_profile(Some(name), Default::default())?;
            }
            Ok(())
        })
        .unwrap();
    service.preview("VRChat-Client-ABC123", "avtr_demo", "Demo Avatar");
    Setup {
        _rt: rt,
        _dir: dir,
        service,
    }
}

/// Renders a few frames (the first ones settle layout) and returns the last one.
fn render(mut ui: MainUi, width: usize, height: usize, ppp: f32) -> (Vec<u8>, Duration) {
    let ctx = egui::Context::default();
    let mut renderer = SoftwareRenderer::new(width, height);
    let mut last = Duration::ZERO;
    for frame in 0..6 {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width as f32 / ppp, height as f32 / ppp),
            )),
            time: Some(frame as f64 * 0.033),
            ..Default::default()
        };
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(ppp);
        let start = Instant::now();
        let out = ctx.run(input, |ctx| ui.show(ctx));
        renderer.update_textures(&out.textures_delta);
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        renderer.render(&prims, out.pixels_per_point);
        last = start.elapsed();
    }
    (renderer.pixels_mut().to_vec(), last)
}

fn check(name: &str, (pixels, time): (Vec<u8>, Duration), width: usize, height: usize) {
    println!("{name}: {time:?}");
    if let Some(dir) = std::env::var_os("PSSTATES_RENDER_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        image::save_buffer(
            path,
            &pixels,
            width as u32,
            height as u32,
            image::ColorType::Rgba8,
        )
        .unwrap();
    }
    // Something other than the background colour was drawn (text, panels).
    let drawn = pixels.chunks(4).filter(|p| p[..3] != BACKGROUND).count();
    assert!(drawn > 5000, "{name}: only {drawn} non-background pixels");
}

#[test]
fn renders_overlay_and_desktop_views() {
    let s = setup();
    // The VR dashboard: 1920x1200 px at scale 2.0 (960x600 points).
    let (w, h, ppp) = (1920, 1200, 2.0);
    check(
        "overlay_profiles",
        render(MainUi::new(s.service.clone()).overlay_mode(), w, h, ppp),
        w,
        h,
    );
    check(
        "overlay_profiles_edit",
        render(
            MainUi::new(s.service.clone())
                .overlay_mode()
                .with_edit_mode(true),
            w,
            h,
            ppp,
        ),
        w,
        h,
    );
    check(
        "overlay_settings",
        render(
            MainUi::new(s.service.clone())
                .overlay_mode()
                .on_settings_tab(),
            w,
            h,
            ppp,
        ),
        w,
        h,
    );
    // The desktop window.
    let (w, h, ppp) = (1100, 720, 1.0);
    check(
        "desktop_profiles",
        render(
            MainUi::new(s.service.clone()).without_legacy_prompt(),
            w,
            h,
            ppp,
        ),
        w,
        h,
    );
    check(
        "desktop_settings",
        render(
            MainUi::new(s.service.clone())
                .without_legacy_prompt()
                .on_settings_tab(),
            w,
            h,
            ppp,
        ),
        w,
        h,
    );
    s.service.shutdown();
}

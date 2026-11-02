//! 3D objects on the GPU (ADR-0040): lit solids with soft shadows on a catching floor, the
//! faces' own materials and printed pictures, glTF models with their materials, sorting
//! among 3D layers, the depth of field, determinism, and preview matching export.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "objects_support/mod.rs"]
mod objects_support;
mod support;

use fosscut_model::Id;
use fosscut_render::color::TransferFunction;
use fosscut_render::objects::{ModelData, fixture_glb};
use fosscut_render::plan::{LayerSource, RenderPlan, plan_frame};
use fosscut_render::{ExportGraph, Image, PreviewGraph, Renderer};
use fosscut_time::{Flicks, Rate};
use objects_support::*;

const SIZE: (u32, u32) = (640, 360);

fn at(project: &fosscut_model::Project, timeline: &Id) -> RenderPlan {
    plan_frame(project, timeline, Rate::FPS_30.frames_to_flicks(10)).unwrap()
}

fn render(renderer: &mut Renderer, plan: &RenderPlan, provider: &mut Provider) -> Image {
    renderer
        .render_image(plan, provider, SIZE, TransferFunction::Bt709)
        .unwrap()
}

fn house_provider() -> Provider {
    Provider::new().with_house(ModelData::from_slice(&fixture_glb(Some(&checker_png()))).unwrap())
}

fn rgb(image: &Image, x: u32, y: u32) -> [i32; 3] {
    let p = image.pixel(x, y);
    [i32::from(p[0]), i32::from(p[1]), i32::from(p[2])]
}

fn luma(image: &Image, x: u32, y: u32) -> i32 {
    let [r, g, b] = rgb(image, x, y);
    (2 * r + 5 * g + b) / 8
}

/// Writes every scene to `FOSSCUT_OBJECTS_DUMP` when it is set, for looking at them.
#[test]
fn dump_scenes_for_review() {
    let Some(dir) = std::env::var_os("FOSSCUT_OBJECTS_DUMP") else {
        return;
    };
    let Some(mut renderer) = support::renderer() else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let scenes: Vec<(&str, (fosscut_model::Project, Id))> = vec![
        ("shipping-box", shipping_box()),
        ("city", city()),
        ("extruded-title", extruded_title()),
        ("between-cards", between_cards()),
        ("defocused", defocused()),
        ("house", house()),
    ];
    for (name, (project, timeline)) in scenes {
        let plan = at(&project, &timeline);
        let start = std::time::Instant::now();
        let image = renderer
            .render_image(
                &plan,
                &mut house_provider(),
                (1920, 1080),
                TransferFunction::Bt709,
            )
            .unwrap();
        eprintln!("{name}: {:?} at 1080p (first frame)", start.elapsed());
        let start = std::time::Instant::now();
        let image2 = renderer
            .render_image(
                &plan,
                &mut house_provider(),
                (1920, 1080),
                TransferFunction::Bt709,
            )
            .unwrap();
        eprintln!("{name}: {:?} at 1080p (again)", start.elapsed());
        assert_eq!(image, image2);
        // What the objects cost: the same frame with and without them, ten times each.
        let mut bare = plan.clone();
        bare.layers
            .retain(|l| !matches!(l.source, LayerSource::Model(_)));
        let mut time = |plan: &RenderPlan| {
            let start = std::time::Instant::now();
            for _ in 0..10 {
                renderer
                    .render_image(
                        plan,
                        &mut house_provider(),
                        (1920, 1080),
                        TransferFunction::Bt709,
                    )
                    .unwrap();
            }
            start.elapsed() / 10
        };
        let (with, without) = (time(&plan), time(&bare));
        eprintln!("{name}: {with:?} a 1080p frame with its objects, {without:?} without");
        let file = std::fs::File::create(dir.join(format!("{name}.png"))).unwrap();
        let mut encoder =
            png::Encoder::new(std::io::BufWriter::new(file), image.width, image.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&image.rgba)
            .unwrap();
    }
}

#[test]
fn objects_are_planned_in_the_scene() {
    let (project, timeline) = between_cards();
    let plan = at(&project, &timeline);
    let kinds: Vec<&str> = plan
        .layers
        .iter()
        .map(|l| match &l.source {
            LayerSource::Model(_) => "object",
            LayerSource::Frame(source) if source.asset == support::asset(CORAL) => "coral",
            LayerSource::Frame(source) if source.asset == support::asset(TEAL) => "teal",
            _ => "other",
        })
        .collect();
    // Farthest first: the teal card, the ring, then the coral card in front, although the
    // ring is on the top track.
    assert_eq!(kinds, vec!["other", "teal", "object", "coral"]);
    let object = plan
        .layers
        .iter()
        .find(|l| matches!(l.source, LayerSource::Model(_)))
        .unwrap();
    let space = object.space.as_ref().unwrap();
    assert!(space.scene, "objects are always in the 3D scene");
    assert!(object.transform.three_d);
}

#[test]
fn a_lit_box_casts_a_soft_shadow_on_its_catcher() {
    let Some(mut renderer) = support::renderer() else {
        return;
    };
    let (project, timeline) = shipping_box();
    let plan = at(&project, &timeline);
    let image = render(&mut renderer, &plan, &mut Provider::new());
    // Without the box, only the planks show.
    let mut bare = plan.clone();
    bare.layers
        .retain(|l| !matches!(l.source, LayerSource::Model(_)));
    let floor = render(&mut renderer, &bare, &mut Provider::new());
    // The box is orange where it stands.
    let [r, g, b] = rgb(&image, 230, 150);
    assert!(r > g + 30 && g > b, "box {:?}", [r, g, b]);
    // Its shadow falls to the lower right, away from the light on the upper left, and
    // darkens the planks there; far from the box the planks are untouched.
    let darkened = (0..SIZE.0)
        .flat_map(|x| (0..SIZE.1).map(move |y| (x, y)))
        .filter(|&(x, y)| luma(&image, x, y) + 12 < luma(&floor, x, y))
        .collect::<Vec<_>>();
    assert!(!darkened.is_empty());
    let right = darkened.iter().filter(|(x, _)| *x > 320).count();
    assert!(right * 2 > darkened.len(), "{right} of {}", darkened.len());
    for (x, y) in [(10, 10), (630, 10), (10, 350)] {
        assert!(
            (luma(&image, x, y) - luma(&floor, x, y)).abs() <= 1,
            "({x}, {y})"
        );
    }
    // The shadow fades out: there are pixels darkened by a little and by a lot.
    let depth: Vec<i32> = darkened
        .iter()
        .map(|&(x, y)| luma(&floor, x, y) - luma(&image, x, y))
        .collect();
    assert!(depth.iter().any(|d| *d < 25) && depth.iter().any(|d| *d > 35));
}

#[test]
fn the_front_face_prints_its_picture() {
    let Some(mut renderer) = support::renderer() else {
        return;
    };
    let (mut project, timeline) = shipping_box();
    let plan = at(&project, &timeline);
    let printed = render(&mut renderer, &plan, &mut Provider::new());
    // The same box without the picture.
    for track in &mut project.timelines.get_mut(&timeline).unwrap().tracks {
        for clip in &mut track.clips {
            if let fosscut_model::ClipContent::Model { model } = &mut clip.content {
                model.faces = fosscut_model::Faces::default();
            }
        }
    }
    let plain = render(
        &mut renderer,
        &at(&project, &timeline),
        &mut Provider::new(),
    );
    let changed = (0..SIZE.0)
        .flat_map(|x| (0..SIZE.1).map(move |y| (x, y)))
        .filter(|&(x, y)| luma(&plain, x, y) - luma(&printed, x, y) > 40)
        .count();
    // The dark ring and dot cover a part of the front face, not the whole box.
    assert!(changed > 200 && changed < 6000, "{changed}");
}

#[test]
fn objects_sort_between_3d_layers() {
    let Some(mut renderer) = support::renderer() else {
        return;
    };
    let (project, timeline) = between_cards();
    let image = render(
        &mut renderer,
        &at(&project, &timeline),
        &mut Provider::new(),
    );
    // The ring's left side is behind the coral card; its right side over the teal card.
    let coral = rgb(&image, 220, 180);
    assert!(coral[0] > coral[2] + 40, "coral card in front: {coral:?}");
    let ring = (330..470)
        .flat_map(|x| (120..240).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            let [r, g, b] = rgb(&image, x, y);
            // The grey metal of the ring, not the teal card.
            (r - b).abs() < 25 && (g - b).abs() < 25 && r > 70
        })
        .count();
    assert!(ring > 300, "{ring}");
}

#[test]
fn the_depth_of_field_blurs_a_far_object() {
    let Some(mut renderer) = support::renderer() else {
        return;
    };
    let (mut project, timeline) = defocused();
    let soft = render(
        &mut renderer,
        &at(&project, &timeline),
        &mut Provider::new(),
    );
    for track in &mut project.timelines.get_mut(&timeline).unwrap().tracks {
        for clip in &mut track.clips {
            if let fosscut_model::ClipContent::Camera(camera) = &mut clip.content {
                camera.depth_of_field = false;
            }
        }
    }
    let sharp = render(
        &mut renderer,
        &at(&project, &timeline),
        &mut Provider::new(),
    );
    // Across the sphere's left edge, the sharp picture jumps where the soft one ramps.
    let steepest = |image: &Image| {
        let y = 160;
        (340..520)
            .map(|x| (luma(image, x + 1, y) - luma(image, x, y)).abs())
            .max()
            .unwrap()
    };
    assert!(
        steepest(&sharp) > 2 * steepest(&soft),
        "{} {}",
        steepest(&sharp),
        steepest(&soft)
    );
    // The card in focus stays sharp.
    assert_eq!(rgb(&soft, 200, 220), rgb(&sharp, 200, 220));
}

#[test]
fn gltf_models_draw_with_their_materials() {
    let Some(mut renderer) = support::renderer() else {
        return;
    };
    let (project, timeline) = house();
    let image = render(
        &mut renderer,
        &at(&project, &timeline),
        &mut house_provider(),
    );
    // The red roof above, the checkered walls below it.
    let red = (0..SIZE.0)
        .flat_map(|x| (0..SIZE.1).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            let [r, g, b] = rgb(&image, x, y);
            r > 2 * g && r > 2 * b && r > 60
        })
        .count();
    let blue = (0..SIZE.0)
        .flat_map(|x| (0..SIZE.1).map(move |y| (x, y)))
        .filter(|&(x, y)| {
            let [r, g, b] = rgb(&image, x, y);
            b > r + 50 && b > g + 20
        })
        .count();
    assert!(red > 500, "roof {red}");
    assert!(blue > 300, "checkered walls {blue}");
    // Without the model loaded, the frame fails rather than leaving it out.
    let missing = renderer.render_image(
        &at(&project, &timeline),
        &mut Provider::new(),
        SIZE,
        TransferFunction::Bt709,
    );
    assert!(missing.is_err());
}

#[test]
fn a_frame_is_the_same_alone_and_after_others() {
    let Some(mut renderer) = support::renderer() else {
        return;
    };
    let (project, timeline) = extruded_title();
    let plan = at(&project, &timeline);
    let first = render(&mut renderer, &plan, &mut Provider::new());
    for (project, timeline) in [city(), shipping_box()] {
        render(
            &mut renderer,
            &at(&project, &timeline),
            &mut house_provider(),
        );
    }
    let again = render(&mut renderer, &plan, &mut Provider::new());
    assert_eq!(first, again);
    let Some(mut fresh) = support::renderer() else {
        return;
    };
    assert_eq!(first, render(&mut fresh, &plan, &mut Provider::new()));
}

fn half(image: &Image) -> Image {
    let (w, h) = (image.width / 2, image.height / 2);
    let bt709 = TransferFunction::Bt709;
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let linear: f32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                    .iter()
                    .map(|(dx, dy)| {
                        let code = image.pixel(2 * x + dx, 2 * y + dy)[c];
                        bt709.decode(f32::from(code) / 255.0)
                    })
                    .sum::<f32>()
                    / 4.0;
                rgba.push((bt709.encode(linear).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }
    Image {
        width: w,
        height: h,
        rgba,
    }
}

#[test]
fn preview_matches_export() {
    let Some(mut renderer) = support::renderer() else {
        return;
    };
    for (name, (project, timeline)) in [
        ("shipping box", shipping_box()),
        ("city", city()),
        ("house", house()),
    ] {
        let export = ExportGraph::new(&project, &timeline).unwrap();
        let preview = PreviewGraph::new(&project, &timeline, (960, 960)).unwrap();
        let time = Rate::FPS_30.frames_to_flicks(10);
        let full = export
            .render(&mut renderer, &project, &mut house_provider(), time)
            .unwrap();
        let small = preview
            .grab(&mut renderer, &project, &mut house_provider(), time)
            .unwrap();
        // Each resolution has its own shadow map and multisampled edges; 38 dB stays
        // invisible while a missing or misplaced object or shadow drops far below it.
        let psnr = half(&full).psnr(&small).unwrap();
        eprintln!("{name}: preview against export {psnr:.1} dB");
        assert!(psnr > 38.0, "{name}: PSNR {psnr:.1} dB");
    }
    let _ = Flicks::ZERO;
}

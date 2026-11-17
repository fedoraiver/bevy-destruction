use bevy::{
    asset::RenderAssetUsages,
    camera::{RenderTarget, visibility::RenderLayers},
    core_pipeline::prepass::DepthPrepass,
    gltf::GltfMaterial,
    mesh::PrimitiveTopology,
    prelude::*,
    render::{
        render_resource::TextureFormat,
        storage::ShaderBuffer,
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
};

use crate::{
    DemoOptions,
    contact_mask::ContactMaskBuilder,
    fracture_detail::{ContactPatch, contact_surface_mesh},
    material::{BuildingMaterial, building_material, surface_coordinates},
};

#[cfg(test)]
mod tests;

pub struct ContactSmokePlugin;
impl Plugin for ContactSmokePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ContactState>()
            .add_systems(Startup, setup)
            .add_systems(Update, drive);
    }
}

const WIDTH: usize = 512;
const HEIGHT: usize = 512;
const MODES: [&str; 5] = [
    "grid-only",
    "fracture-reference",
    "source-mask",
    "legacy-overlay-control",
    "source-mask-repeat",
];

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    distance: f32,
    angle: f32,
    pose: usize,
    prepass: bool,
}
impl Case {
    fn camera(self) -> Transform {
        Transform::from_translation(Vec3::new(
            self.angle.sin() * self.distance,
            0.0,
            self.angle.cos() * self.distance,
        ))
        .looking_at(Vec3::ZERO, Vec3::Y)
    }
    fn fov(self) -> f32 {
        2.0 * (1.7 / self.distance).atan()
    }
    fn occluder_z(self) -> f32 {
        if self.angle.cos() > 0.0 { 0.03 } else { -0.03 }
    }
    fn label(self) -> String {
        format!(
            "{}-{}",
            if self.prepass {
                "prepass-on"
            } else {
                "prepass-off"
            },
            self.name
        )
    }
}
fn cases() -> Vec<Case> {
    [false, true]
        .into_iter()
        .flat_map(|prepass| {
            [
                Case {
                    name: "near-rest",
                    distance: 3.0,
                    angle: 0.0,
                    pose: 0,
                    prepass,
                },
                Case {
                    name: "far-translated",
                    distance: 100.0,
                    angle: 0.0,
                    pose: 1,
                    prepass,
                },
                Case {
                    name: "grazing-rotated",
                    distance: 6.0,
                    angle: 72_f32.to_radians(),
                    pose: 2,
                    prepass,
                },
                Case {
                    name: "backface-rotated",
                    distance: 6.0,
                    angle: std::f32::consts::PI,
                    pose: 2,
                    prepass,
                },
            ]
        })
        .collect()
}
fn motion(pose: usize) -> Transform {
    match pose {
        0 => Transform::IDENTITY,
        1 => Transform::from_xyz(7.137, -2.463, 3.147),
        _ => Transform::from_xyz(37.173, 13.287, -22.419).with_rotation(Quat::from_euler(
            EulerRot::XYZ,
            0.37,
            0.61,
            -0.29,
        )),
    }
}
fn rest() -> Transform {
    Transform::from_xyz(2.137, 0.263, -0.419).with_rotation(Quat::from_euler(
        EulerRot::XYZ,
        0.23,
        0.47,
        -0.31,
    ))
}

#[derive(Default)]
struct Regions {
    contact: Vec<usize>,
    grid: Vec<usize>,
    occluder: Vec<usize>,
}
fn regions(case: Case) -> Regions {
    let camera = case.camera();
    let tangent = (case.fov() * 0.5).tan();
    let mut result = Regions::default();
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            if y < 50 && x < 115 {
                continue;
            }
            let direction = camera.rotation
                * Vec3::new(
                    (2.0 * (x as f32 + 0.5) / WIDTH as f32 - 1.0) * tangent,
                    (1.0 - 2.0 * (y as f32 + 0.5) / HEIGHT as f32) * tangent,
                    -1.0,
                );
            if direction.z.abs() < 1e-7 {
                continue;
            }
            let intersect = |z: f32| {
                let t = (z - camera.translation.z) / direction.z;
                (t > 0.0).then_some(camera.translation + direction * t)
            };
            let Some(front) = intersect(case.occluder_z()) else {
                continue;
            };
            let fx = (front.x - 0.15).abs();
            let fy = (front.y - 0.1).abs();
            let index = (y * WIDTH + x) * 4;
            if fx < 0.22 && fy < 0.29 {
                result.occluder.push(index);
                continue;
            }
            if fx < 0.34 && fy < 0.41 {
                continue;
            }
            let Some(point) = intersect(0.0) else {
                continue;
            };
            let x = point.x.abs();
            let y = point.y.abs();
            if x > 1.85 || y > 1.85 {
                continue;
            }
            if x < 0.86 && y < 0.86 {
                result.contact.push(index);
            } else if x > 1.14 || y > 1.14 {
                result.grid.push(index);
            }
        }
    }
    result
}

#[derive(Resource, Default)]
struct ContactState {
    target: Handle<Image>,
    parent: Option<Entity>,
    source: Option<Entity>,
    overlay: Option<Entity>,
    occluder: Option<Entity>,
    camera: Option<Entity>,
    light: Option<Entity>,
    plain_mesh: Handle<Mesh>,
    masked_mesh: Handle<Mesh>,
    grid_material: Handle<BuildingMaterial>,
    masked_material: Handle<BuildingMaterial>,
    markers: Vec<Entity>,
    marker_materials: Vec<Handle<StandardMaterial>>,
    cases: Vec<Case>,
    regions: Vec<Regions>,
    phase: usize,
    applied: Option<usize>,
    hold: u32,
    frames: u32,
    pending: bool,
    captures: Vec<Vec<u8>>,
    rejected_readbacks: usize,
    errors: Vec<String>,
    finished: bool,
}

fn rectangle(min: Vec2, max: Vec2, z: f32) -> Mesh {
    let a = [min.x, min.y, z];
    let b = [max.x, min.y, z];
    let c = [max.x, max.y, z];
    let d = [min.x, max.y, z];
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![a, b, c, a, c, d])
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 6])
}
fn source_plane() -> Mesh {
    rectangle(Vec2::splat(-2.0), Vec2::splat(2.0), 0.0)
}
fn patch() -> ContactPatch {
    let a = Vec3::new(-1.0, -1.0, -0.00001);
    let b = Vec3::new(1.0, -1.0, -0.00001);
    let c = Vec3::new(1.0, 1.0, -0.00001);
    let d = Vec3::new(-1.0, 1.0, -0.00001);
    ContactPatch {
        id: "contact-probe".into(),
        bond_id: "contact-probe".into(),
        a: "source".into(),
        b: "neighbor".into(),
        normal: Vec3::Z,
        triangles: vec![[a, b, c], [a, c, d]],
        area: 4.0,
        rims: Vec::new(),
    }
}

#[allow(clippy::too_many_arguments)]
fn setup(
    mut commands: Commands,
    options: Res<DemoOptions>,
    mut state: ResMut<ContactState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BuildingMaterial>>,
    mut standard: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
) {
    let Some(directory) = &options.contact_smoke_output else {
        return;
    };
    std::fs::create_dir_all(directory).expect("Create contact smoke output");
    state.cases = cases();
    state.regions = state.cases.iter().copied().map(regions).collect();
    state.target = images.add(Image::new_target_texture(
        WIDTH as u32,
        HEIGHT as u32,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let rest = rest();
    let inverse = Transform::from_matrix(rest.to_matrix().inverse());
    let parent = commands.spawn((rest, Visibility::Visible)).id();
    state.parent = Some(parent);
    let imported = GltfMaterial {
        double_sided: true,
        cull_mode: None,
        ..default()
    };
    let grid = building_material(Some(&imported), false, "concrete");
    assert!(grid.base.double_sided && grid.base.cull_mode.is_none());
    state.grid_material = materials.add(grid);
    let mut fracture = building_material(Some(&imported), true, "concrete");
    fracture.base.depth_bias = 2.0;
    let fracture = materials.add(fracture);
    let source = surface_coordinates(source_plane().transformed_by(inverse), rest);
    let mut builder = ContactMaskBuilder::default();
    let masked = builder
        .mask_mesh(source.clone(), &[patch()], "source")
        .expect("Canonical source contact mask");
    let mut masked_material = building_material(Some(&imported), false, "concrete");
    masked_material.extension.contacts = buffers.add(ShaderBuffer::from(builder.data().to_vec()));
    state.masked_material = materials.add(masked_material);
    state.masked_mesh = meshes.add(masked);
    state.plain_mesh = meshes.add(source);
    state.source = Some(
        commands
            .spawn((
                Name::new("Contact probe source grid"),
                Mesh3d(state.plain_mesh.clone()),
                MeshMaterial3d(state.grid_material.clone()),
                Transform::IDENTITY,
                Visibility::Inherited,
                ChildOf(parent),
                RenderLayers::layer(1),
            ))
            .id(),
    );
    let overlay = surface_coordinates(
        contact_surface_mesh(&[patch()], "source").unwrap(),
        Transform::IDENTITY,
    )
    .transformed_by(inverse);
    state.overlay = Some(
        commands
            .spawn((
                Name::new("Legacy contact negative control / rough reference"),
                Mesh3d(meshes.add(overlay)),
                MeshMaterial3d(fracture),
                Transform::IDENTITY,
                Visibility::Hidden,
                ChildOf(parent),
                RenderLayers::layer(1),
            ))
            .id(),
    );
    let occluder = standard.add(StandardMaterial {
        base_color: Color::srgb(0.05, 0.8, 0.1),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    state.occluder = Some(
        commands
            .spawn((
                Name::new("Front contact occlusion witness"),
                Mesh3d(
                    meshes.add(
                        rectangle(Vec2::new(-0.12, -0.24), Vec2::new(0.42, 0.44), 0.0)
                            .transformed_by(inverse),
                    ),
                ),
                MeshMaterial3d(occluder),
                Transform::IDENTITY,
                Visibility::Inherited,
                ChildOf(parent),
                RenderLayers::layer(1),
            ))
            .id(),
    );
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                order: -1,
                clear_color: Color::srgb(0.02, 0.025, 0.03).into(),
                ..default()
            },
            Projection::Perspective(PerspectiveProjection {
                fov: std::f32::consts::FRAC_PI_3,
                near: 0.05,
                far: 500.0,
                ..default()
            }),
            RenderTarget::Image(state.target.clone().into()),
            state.cases[0].camera(),
            AmbientLight {
                brightness: 0.0,
                ..default()
            },
            RenderLayers::layer(1),
            Msaa::Off,
        ))
        .id();
    state.camera = Some(camera);
    state.light = Some(
        commands
            .spawn((
                DirectionalLight {
                    illuminance: 6500.0,
                    shadow_maps_enabled: false,
                    ..default()
                },
                light_rest(),
                RenderLayers::layer(1),
            ))
            .id(),
    );
    state.marker_materials = [Color::BLACK, Color::WHITE]
        .into_iter()
        .map(|color| {
            standard.add(StandardMaterial {
                base_color: color,
                unlit: true,
                double_sided: true,
                cull_mode: None,
                ..default()
            })
        })
        .collect();
    let marker = meshes.add(rectangle(Vec2::splat(-0.5), Vec2::splat(0.5), 0.0));
    for _ in 0..6 {
        let entity = commands
            .spawn((
                Mesh3d(marker.clone()),
                MeshMaterial3d(state.marker_materials[0].clone()),
                Transform::IDENTITY,
                ChildOf(camera),
                RenderLayers::layer(1),
            ))
            .id();
        state.markers.push(entity);
    }
}
fn light_rest() -> Transform {
    Transform::from_xyz(-3.0, 4.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y)
}

#[derive(Debug, serde::Serialize)]
struct PixelDifference {
    mean_rgb_error: f64,
    changed_pixel_fraction: f64,
    pixels: usize,
}
fn difference(reference: &[u8], actual: &[u8], indices: &[usize]) -> PixelDifference {
    let mut total = 0_u64;
    let mut changed = 0;
    for &index in indices {
        let delta = (0..3)
            .map(|channel| u64::from(reference[index + channel].abs_diff(actual[index + channel])))
            .sum::<u64>();
        total += delta;
        changed += usize::from(delta > 24);
    }
    PixelDifference {
        mean_rgb_error: total as f64 / (indices.len() * 3).max(1) as f64,
        changed_pixel_fraction: changed as f64 / indices.len().max(1) as f64,
        pixels: indices.len(),
    }
}
fn brightness(pixels: &[u8], indices: &[usize]) -> f64 {
    indices
        .iter()
        .map(|&index| (0..3).map(|c| u64::from(pixels[index + c])).sum::<u64>())
        .sum::<u64>() as f64
        / (indices.len() * 3).max(1) as f64
}
fn mean_color(pixels: &[u8], indices: &[usize]) -> [f64; 3] {
    std::array::from_fn(|channel| {
        indices
            .iter()
            .map(|&index| u64::from(pixels[index + channel]))
            .sum::<u64>() as f64
            / indices.len().max(1) as f64
    })
}
fn witness(pixels: &[u8]) -> usize {
    let mut value = 0;
    for bit in 0..6 {
        let x = 20 + bit * 12;
        let mut sum = 0_u32;
        for y in 19..=21 {
            for x in x - 1..=x + 1 {
                let i = (y * WIDTH + x) * 4;
                sum += u32::from(pixels[i]) + u32::from(pixels[i + 1]) + u32::from(pixels[i + 2]);
            }
        }
        if sum / (9 * 3) > 100 {
            value |= 1 << bit;
        }
    }
    value
}
fn failed(metric: &PixelDifference) -> bool {
    metric.mean_rgb_error > 3.0 || metric.changed_pixel_fraction > 0.02
}

fn drive(world: &mut World) {
    let Some(directory) = world.resource::<DemoOptions>().contact_smoke_output.clone() else {
        return;
    };
    let mut state = world.remove_resource::<ContactState>().unwrap();
    tick(world, &mut state, &directory);
    world.insert_resource(state);
}
fn tick(world: &mut World, state: &mut ContactState, directory: &std::path::Path) {
    if state.finished {
        return;
    }
    state.frames += 1;
    let count = state.cases.len() * MODES.len();
    if state.frames > 4000 {
        state
            .errors
            .push("Contact matrix timed out waiting for phase-matched GPU readback".into());
        state.phase = count;
    }
    if state.phase == count {
        finish(world, state, directory);
        return;
    }
    let case_index = state.phase / MODES.len();
    let mode = state.phase % MODES.len();
    let case = state.cases[case_index];
    if state.applied != Some(state.phase) {
        let motion = motion(case.pose);
        let rest = rest();
        *world.get_mut::<Transform>(state.parent.unwrap()).unwrap() =
            Transform::from_matrix(motion.to_matrix() * rest.to_matrix());
        *world.get_mut::<Transform>(state.camera.unwrap()).unwrap() =
            Transform::from_matrix(motion.to_matrix() * case.camera().to_matrix());
        *world.get_mut::<Projection>(state.camera.unwrap()).unwrap() =
            Projection::Perspective(PerspectiveProjection {
                fov: case.fov(),
                near: 0.05,
                far: 500.0,
                ..default()
            });
        if case.prepass {
            world.entity_mut(state.camera.unwrap()).insert(DepthPrepass);
        } else {
            world
                .entity_mut(state.camera.unwrap())
                .remove::<DepthPrepass>();
        }
        let light = Transform::from_xyz(-3.0, 4.0, if case.angle.cos() > 0.0 { 5.0 } else { -5.0 })
            .looking_at(Vec3::ZERO, Vec3::Y);
        *world.get_mut::<Transform>(state.light.unwrap()).unwrap() =
            Transform::from_matrix(motion.to_matrix() * light.to_matrix());
        world
            .get_mut::<Transform>(state.occluder.unwrap())
            .unwrap()
            .translation = rest.rotation.inverse() * (Vec3::Z * case.occluder_z());
        let masked = mode == 2 || mode == 4;
        world.get_mut::<Mesh3d>(state.source.unwrap()).unwrap().0 = if masked {
            state.masked_mesh.clone()
        } else {
            state.plain_mesh.clone()
        };
        world
            .get_mut::<MeshMaterial3d<BuildingMaterial>>(state.source.unwrap())
            .unwrap()
            .0 = if masked {
            state.masked_material.clone()
        } else {
            state.grid_material.clone()
        };
        *world.get_mut::<Visibility>(state.source.unwrap()).unwrap() = if mode == 1 {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        *world.get_mut::<Visibility>(state.overlay.unwrap()).unwrap() = if mode == 1 || mode == 3 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let tangent = (case.fov() * 0.5).tan();
        for (bit, &entity) in state.markers.iter().enumerate() {
            let x = 20.0 + bit as f32 * 12.0;
            *world.get_mut::<Transform>(entity).unwrap() = Transform::from_xyz(
                (2.0 * x / WIDTH as f32 - 1.0) * tangent,
                (1.0 - 40.0 / HEIGHT as f32) * tangent,
                -1.0,
            )
            .with_scale(Vec3::splat(8.0 / WIDTH as f32 * 2.0 * tangent));
            world
                .get_mut::<MeshMaterial3d<StandardMaterial>>(entity)
                .unwrap()
                .0 =
                state.marker_materials[usize::from((state.phase + 1) & (1 << bit) != 0)].clone();
        }
        state.applied = Some(state.phase);
        state.hold = 0;
    }
    state.hold += 1;
    if state.pending || state.hold < if state.phase == 0 { 180 } else { 12 } {
        return;
    }
    let phase = state.phase;
    let path = directory.join(format!(
        "{:02}-{}-{}.png",
        phase + 1,
        case.label(),
        MODES[mode]
    ));
    let rejected = directory.join("rejected-first-readback.png");
    world
        .spawn(Screenshot::image(state.target.clone()))
        .observe(
            move |capture: On<ScreenshotCaptured>, mut state: ResMut<ContactState>| {
                state.pending = false;
                if state.finished || state.phase != phase {
                    return;
                }
                match capture.image.clone().try_into_dynamic() {
                    Ok(image) => {
                        let pixels = image.to_rgba8().into_raw();
                        let valid =
                            pixels.len() == WIDTH * HEIGHT * 4 && witness(&pixels) == phase + 1;
                        let visible = mode != 0 && mode != 1
                            || (valid
                                && brightness(&pixels, &state.regions[case_index].contact) > 25.0);
                        if !valid || !visible {
                            if state.rejected_readbacks == 0 {
                                let _ = image.to_rgb8().save(&rejected);
                            }
                            state.rejected_readbacks += 1;
                            state.hold = 5;
                            return;
                        }
                        if let Err(error) = image.to_rgb8().save(&path) {
                            state.errors.push(format!("Contact screenshot: {error}"));
                        }
                        state.captures.push(pixels);
                        state.phase += 1;
                    }
                    Err(error) => {
                        state.errors.push(format!("Contact readback: {error}"));
                        state.phase = state.cases.len() * MODES.len();
                    }
                }
            },
        );
    state.pending = true;
}

fn finish(world: &mut World, state: &mut ContactState, directory: &std::path::Path) {
    let mut results = Vec::new();
    let mut negative_near = 0;
    if state.captures.len() != state.cases.len() * MODES.len() {
        state
            .errors
            .push("Contact matrix did not capture every phase".into());
    }
    for (index, &case) in state.cases.iter().enumerate() {
        let start = index * MODES.len();
        if state.captures.len() < start + MODES.len() {
            break;
        }
        let r = &state.regions[index];
        if [r.contact.len(), r.grid.len(), r.occluder.len()]
            .into_iter()
            .any(|n| n < 64)
        {
            state
                .errors
                .push(format!("{} has insufficient region pixels", case.label()));
        }
        let grid = &state.captures[start];
        let reference = &state.captures[start + 1];
        let sensitivity = difference(grid, reference, &r.contact);
        if sensitivity.mean_rgb_error < 12.0 || sensitivity.changed_pixel_fraction < 0.6 {
            state.errors.push(format!(
                "{} reference cannot distinguish contact/grid",
                case.label()
            ));
        }
        let occluder_color = mean_color(grid, &r.occluder);
        if occluder_color[1] < 60.0
            || occluder_color[1] - occluder_color[0].max(occluder_color[2]) < 30.0
        {
            state.errors.push(format!(
                "{} forward occluder is not visibly green",
                case.label()
            ));
        }
        let mut positive = Vec::new();
        for mode in [2, 4] {
            let contact = difference(reference, &state.captures[start + mode], &r.contact);
            let outside = difference(grid, &state.captures[start + mode], &r.grid);
            let occlusion = difference(grid, &state.captures[start + mode], &r.occluder);
            if failed(&contact) {
                state.errors.push(format!(
                    "{} {} source grid leaks inside contact",
                    case.label(),
                    MODES[mode]
                ));
            }
            if failed(&outside) {
                state.errors.push(format!(
                    "{} {} changed exterior grid",
                    case.label(),
                    MODES[mode]
                ));
            }
            if failed(&occlusion) {
                state.errors.push(format!(
                    "{} {} penetrated front occluder",
                    case.label(),
                    MODES[mode]
                ));
            }
            positive.push(serde_json::json!({"mode":MODES[mode],"contact":contact,"outside_grid":outside,"front_occluder":occlusion}));
        }
        let legacy = difference(reference, &state.captures[start + 3], &r.contact);
        if case.name == "near-rest" && failed(&legacy) {
            negative_near += 1;
        }
        let stability = difference(
            &state.captures[start + 2],
            &state.captures[start + 4],
            &r.contact,
        );
        if failed(&stability) {
            state.errors.push(format!(
                "{} contact pixels changed between repeated captures",
                case.label()
            ));
        }
        results.push(serde_json::json!({"case":case.label(),"distance_m":case.distance,"fov_rad":case.fov(),"angle_rad":case.angle,"pose":case.pose,"depth_prepass":case.prepass,"double_sided":true,"cull_mode":"None","reference_sensitivity":sensitivity,"occluder_reference_rgb":occluder_color,"positive":positive,"temporal_repeat":stability,"legacy_negative_control":legacy,"legacy_expected_failure":failed(&legacy)}));
    }
    if negative_near != 2 {
        state.errors.push(
            "Legacy overlay negative control did not fail at 3m with both prepass settings".into(),
        );
    }
    let report = serde_json::json!({"single_source_fragment_mask":true,"source_z_m":0.0,"contact_z_m":-0.00001,"near_m":0.05,"far_m":500.0,"captures":state.captures.len(),"rejected_readbacks":state.rejected_readbacks,"cases":results,"legacy_near_failures":negative_near,"errors":state.errors});
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .expect("Write contact matrix report");
    state.finished = true;
    world.write_message(if state.errors.is_empty() {
        AppExit::Success
    } else {
        AppExit::error()
    });
}

use bevy::{
    camera::{RenderTarget, ScalingMode, visibility::RenderLayers},
    prelude::*,
    render::{
        render_resource::TextureFormat,
        view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    },
};

use crate::{
    ProbeOptions,
    material::{BuildingMaterial, building_material, rim_material, surface_coordinates},
};

pub struct MaterialSmokePlugin;

impl Plugin for MaterialSmokePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ProbeState>()
            .add_systems(Startup, setup)
            .add_systems(Update, drive);
    }
}

#[derive(Component)]
struct ProbeCamera;

#[derive(Component)]
struct ProbeLight;

#[derive(Resource, Default)]
struct ProbeState {
    frames: u32,
    hold: u32,
    phase: usize,
    applied_phase: Option<usize>,
    pending: bool,
    target: Handle<Image>,
    pieces: Vec<(Entity, Transform, Handle<Mesh>)>,
    captures: Vec<Vec<u8>>,
    errors: Vec<String>,
    finished: bool,
}

const NAMES: [&str; 4] = [
    "01-rest.png",
    "02-translated.png",
    "03-rotated.png",
    "04-world-locked-control.png",
];

fn camera_rest() -> Transform {
    Transform::from_xyz(0.125, 0.0, 6.0).looking_at(Vec3::new(0.125, 0.0, 0.0), Vec3::Y)
}

fn light_rest() -> Transform {
    Transform::from_xyz(-3.0, 4.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y)
}

fn motion(phase: usize) -> Transform {
    if phase == 0 {
        Transform::IDENTITY
    } else if phase == 1 {
        Transform::from_xyz(7.137, -2.463, 3.147)
    } else {
        Transform::from_xyz(-4.173, 3.287, -2.419).with_rotation(Quat::from_euler(
            EulerRot::XYZ,
            0.37,
            0.61,
            -0.29,
        ))
    }
}

fn setup(
    mut commands: Commands,
    probe_options: Res<ProbeOptions>,
    mut state: ResMut<ProbeState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<BuildingMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(directory) = &probe_options.material_smoke_output else {
        return;
    };
    std::fs::create_dir_all(directory).expect("Create material probe directory");
    state.target = images.add(Image::new_target_texture(
        512,
        256,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let grid = materials.add(building_material(None, false, "concrete"));
    let fracture = materials.add(rim_material("concrete"));
    for (index, x) in [-1.5_f32, 0.0, 1.75].into_iter().enumerate() {
        let rest = Transform::from_xyz(x + 0.137, 0.263, -0.419).with_rotation(Quat::from_euler(
            EulerRot::XYZ,
            0.23,
            0.47,
            -0.31,
        ));
        let local = Transform::from_matrix(rest.to_matrix().inverse());
        let rest_mesh = Mesh::from(Cuboid::new(1.5, 2.0, 0.18))
            .transformed_by(Transform::from_xyz(x, 0.0, 0.0));
        let mesh = if index < 2 {
            surface_coordinates(rest_mesh.transformed_by(local), rest)
        } else {
            surface_coordinates(rest_mesh, Transform::IDENTITY).transformed_by(local)
        };
        let mesh = meshes.add(mesh);
        let entity = commands
            .spawn((
                Name::new("Attached material probe"),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(if index < 2 {
                    grid.clone()
                } else {
                    fracture.clone()
                }),
                rest,
                RenderLayers::layer(1),
            ))
            .id();
        state.pieces.push((entity, rest, mesh));
    }
    commands.spawn((
        Name::new("Material probe camera"),
        Camera3d::default(),
        Camera {
            order: -1,
            clear_color: Color::srgb(0.025, 0.03, 0.035).into(),
            ..default()
        },
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: 3.0,
            },
            ..OrthographicProjection::default_3d()
        }),
        RenderTarget::Image(state.target.clone().into()),
        camera_rest(),
        ProbeCamera,
        AmbientLight {
            brightness: 0.0,
            ..default()
        },
        RenderLayers::layer(1),
        Msaa::Off,
    ));
    commands.spawn((
        Name::new("Material probe light"),
        DirectionalLight {
            illuminance: 6500.0,
            shadow_maps_enabled: false,
            ..default()
        },
        light_rest(),
        RenderLayers::layer(1),
        ProbeLight,
    ));
}

fn compare(reference: &[u8], actual: &[u8]) -> (f64, f64) {
    let mut difference = 0_u64;
    let mut changed = 0;
    for (a, b) in reference
        .as_chunks::<4>()
        .0
        .iter()
        .zip(actual.as_chunks::<4>().0.iter())
    {
        let delta = (0..3)
            .map(|channel| a[channel].abs_diff(b[channel]) as u64)
            .sum::<u64>();
        difference += delta;
        changed += usize::from(delta > 36);
    }
    let pixels = reference.len() / 4;
    (
        difference as f64 / (pixels * 3) as f64,
        changed as f64 / pixels as f64,
    )
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive(
    mut commands: Commands,
    probe_options: Res<ProbeOptions>,
    mut state: ResMut<ProbeState>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut pieces: Query<(&mut Transform, &mut Mesh3d), (Without<ProbeCamera>, Without<ProbeLight>)>,
    mut cameras: Query<&mut Transform, (With<ProbeCamera>, Without<ProbeLight>)>,
    mut lights: Query<&mut Transform, (With<ProbeLight>, Without<ProbeCamera>)>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(directory) = &probe_options.material_smoke_output else {
        return;
    };
    if state.finished {
        return;
    }
    state.frames += 1;
    if state.frames > 2400 {
        state
            .errors
            .push("Material probe timed out waiting for renderer readback".into());
        state.phase = 4;
    }
    if state.phase == 4 {
        let mut comparisons = Vec::new();
        if state.captures.len() == 4 {
            for (index, name) in NAMES.iter().enumerate().skip(1) {
                let (mean, changed_fraction) = compare(&state.captures[0], &state.captures[index]);
                comparisons.push(serde_json::json!({"capture":name,"mean_rgb_error":mean,"changed_pixel_fraction":changed_fraction}));
                if index < 3 && (mean > 0.75 || changed_fraction > 0.02) {
                    state.errors.push(format!(
                        "Attached material changed under rigid motion: {}",
                        name
                    ));
                }
                if index == 3 && mean < 1.5 {
                    state.errors.push("World-locked negative control did not change enough to prove probe sensitivity".into());
                }
            }
        }
        let report = serde_json::json!({"captures":state.captures.len(),"comparisons":comparisons,"errors":state.errors});
        std::fs::write(
            directory.join("report.json"),
            serde_json::to_string_pretty(&report).unwrap(),
        )
        .expect("Write material probe report");
        state.finished = true;
        if state.errors.is_empty() {
            info!("Material probe passed: {report}");
            exit.write(AppExit::Success);
        } else {
            error!("Material probe failed: {report}");
            exit.write(AppExit::error());
        }
        return;
    }
    if state.applied_phase != Some(state.phase) {
        let phase = state.phase;
        let motion = motion(phase);
        for (entity, rest, mesh) in &state.pieces {
            let world = Transform::from_matrix(motion.to_matrix() * rest.to_matrix());
            let (mut transform, mut render_mesh) = pieces.get_mut(*entity).unwrap();
            *transform = world;
            if phase == 3 {
                let mesh = meshes.get(mesh).unwrap().clone();
                render_mesh.0 = meshes.add(surface_coordinates(mesh, world));
            }
        }
        for mut camera in &mut cameras {
            *camera = Transform::from_matrix(motion.to_matrix() * camera_rest().to_matrix());
        }
        for mut light in &mut lights {
            *light = Transform::from_matrix(motion.to_matrix() * light_rest().to_matrix());
        }
        state.applied_phase = Some(phase);
        state.hold = 0;
    }
    state.hold += 1;
    if state.pending || state.hold < if state.phase == 0 { 180 } else { 30 } {
        return;
    }
    let phase = state.phase;
    let name = NAMES[phase];
    commands
        .spawn((
            Name::new("Material smoke screenshot"),
            Screenshot::image(state.target.clone()),
        ))
        .observe(save_to_disk(directory.join(name)))
        .observe(
            move |capture: On<ScreenshotCaptured>, mut state: ResMut<ProbeState>| {
                state.pending = false;
                match capture.image.clone().try_into_dynamic() {
                    Ok(image) => {
                        let pixels = image.into_rgba8().into_raw();
                        let bright = pixels
                            .as_chunks::<4>()
                            .0
                            .iter()
                            .filter(|pixel| {
                                u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2])
                                    > 240
                            })
                            .count();
                        if phase == 0 && bright < pixels.len() / 24 {
                            state.hold = 150;
                            return;
                        }
                        state.captures.push(pixels);
                        state.phase += 1;
                    }
                    Err(error) => {
                        state
                            .errors
                            .push(format!("Material readback image: {error}"));
                        state.phase = 4;
                    }
                }
            },
        );
    state.pending = true;
}

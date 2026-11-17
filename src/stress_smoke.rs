use avian3d::prelude::*;
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};

use crate::{
    DemoOptions,
    asset::{AnchorRecord, BondRecord, ColliderRecord, DestructibleAsset, Manifest},
    fracture_detail::ContactPatch,
    physics::Cluster,
    projectiles::{
        ImpactStats, LaunchRequest, LaunchRequests, PROJECTILE_MASS, PROJECTILE_SPEED,
        ProjectileShape, RigidProjectile,
    },
    scene::{
        BuildingBounds, BuildingInstance, ChunkIdentity, ChunkInstance, DemoCamera, RemovedChunk,
    },
    stress_runtime::{StressSettings, StressState},
};

pub struct StressSmokePlugin;

impl Plugin for StressSmokePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StressSmokeState>().add_systems(
            Update,
            drive
                .after(crate::camera::move_camera)
                .after(crate::scene::instantiate_pending),
        );
    }
}

#[derive(Component)]
struct StressProbe;

#[derive(Resource, Default)]
struct StressSmokeState {
    waiting: u32,
    warmup: u32,
    frame: u32,
    probe: Option<Entity>,
    baseline_utilization: f64,
    overload_utilization: f64,
    overload_damage: f64,
    overload_load: Option<serde_json::Value>,
    asset_max_utilization: f64,
    saw_anchored_overload: bool,
    stress_released: bool,
    fell: bool,
    slept: bool,
    broken_bonds: u64,
    errors: Vec<String>,
    asset_reports: Vec<serde_json::Value>,
    impact_target: Option<Vec3>,
    known_projectiles: Vec<Entity>,
    before_contacts: u64,
    before_impact_breaks: u64,
    sphere_survived: bool,
    cube_survived: bool,
    sphere_contact: bool,
    cube_contact: bool,
    impact_contacts: u64,
    impact_broken_bonds: u64,
    max_impulse: f32,
}

pub(crate) const LEFT_SUPPORT: &str = "a-left-column";
pub(crate) const RIGHT_SUPPORT: &str = "b-right-column";
pub(crate) const BEAM: &str = "c-beam";
pub(crate) const LEFT_BOND: &str = "left-beam";
pub(crate) const RIGHT_BOND: &str = "right-beam";

pub(crate) fn beam_manifest() -> Manifest {
    let mut manifest: Manifest = serde_json::from_str(include_str!(
        "../assets/buildings/rectangle.destruction.json"
    ))
    .unwrap();
    manifest.schema_version = 1;
    manifest.name = "Concrete beam with two narrow supports".into();
    manifest.materials.retain(|id, _| id == "concrete");
    let template = manifest.chunks[0].clone();
    manifest.chunks = [
        (
            LEFT_SUPPORT,
            Vec3::new(-1.0, 0.5, 0.0),
            Vec3::new(0.1, 1.0, 0.1),
        ),
        (
            RIGHT_SUPPORT,
            Vec3::new(1.0, 0.5, 0.0),
            Vec3::new(0.1, 1.0, 0.1),
        ),
        (BEAM, Vec3::new(0.0, 1.1, 0.0), Vec3::new(3.0, 0.2, 0.2)),
    ]
    .into_iter()
    .map(|(id, position, size)| {
        let mut chunk = template.clone();
        chunk.id = id.into();
        chunk.node = id.into();
        chunk.part_id = id.into();
        chunk.kind = if id == BEAM { "beam" } else { "column" }.into();
        chunk.material = "concrete".into();
        chunk.position = position.to_array();
        chunk.rotation = Quat::IDENTITY.to_array();
        chunk.center_of_mass = [0.0; 3];
        chunk.volume = size.element_product();
        chunk.mass = chunk.volume * manifest.materials["concrete"].density;
        chunk.inertia = Mat3::from_diagonal(
            Vec3::new(
                size.y * size.y + size.z * size.z,
                size.x * size.x + size.z * size.z,
                size.x * size.x + size.y * size.y,
            ) * (chunk.mass / 12.0),
        )
        .to_cols_array_2d();
        chunk.colliders = vec![ColliderRecord {
            kind: "convex_hull".into(),
            vertices: [-0.5, 0.5]
                .into_iter()
                .flat_map(|x| {
                    [-0.5, 0.5].into_iter().flat_map(move |y| {
                        [-0.5, 0.5].map(|z| (Vec3::new(x, y, z) * size).to_array())
                    })
                })
                .collect(),
        }];
        chunk
    })
    .collect();
    let second_moment = 0.01 * 0.1 * 0.1 / 12.0;
    manifest.bonds = [
        (LEFT_BOND, LEFT_SUPPORT, -1.0),
        (RIGHT_BOND, RIGHT_SUPPORT, 1.0),
    ]
    .map(|(id, support, x)| BondRecord {
        id: id.into(),
        a: support.into(),
        b: BEAM.into(),
        area: 0.01,
        center: [x, 1.0, 0.0],
        normal: Vec3::Y.to_array(),
        area_covariance: Mat3::from_diagonal(Vec3::new(second_moment, 0.0, second_moment))
            .to_cols_array_2d(),
        state: "rigid".into(),
        patches: Vec::new(),
    })
    .into();
    manifest.anchors = [(LEFT_SUPPORT, -1.0), (RIGHT_SUPPORT, 1.0)]
        .map(|(chunk, x)| AnchorRecord {
            chunk: chunk.into(),
            kind: "fixed".into(),
            position: [x, 0.0, 0.0],
        })
        .into();
    manifest
}

pub(crate) fn spawn_beam(world: &mut World, origin: Vec3) -> Entity {
    let manifest = beam_manifest();
    let asset = world
        .resource_mut::<Assets<DestructibleAsset>>()
        .add(DestructibleAsset {
            manifest: manifest.clone(),
            gltf: Handle::default(),
        });
    let pose = Transform::from_translation(origin);
    let root = world
        .spawn((
            Name::new("Stress acceptance beam"),
            pose,
            GlobalTransform::from(pose),
            Visibility::Visible,
            StressProbe,
        ))
        .id();
    let material = world
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial {
            base_color: Color::srgb(0.72, 0.61, 0.4),
            perceptual_roughness: 0.9,
            ..default()
        });
    let mut instance = BuildingInstance::pending(asset);
    instance.bounds = Some(BuildingBounds {
        min: Vec3::new(-1.5, 0.0, -0.1),
        max: Vec3::new(1.5, 1.2, 0.1),
    });
    instance.topology_dirty = true;
    instance.bonds = manifest
        .bonds
        .iter()
        .map(|bond| (bond.id.clone(), true))
        .collect();
    instance.patches = std::sync::Arc::new(
        manifest
            .bonds
            .iter()
            .map(|bond| {
                let [x, y, z] = bond.center;
                let a = Vec3::new(x - 0.05, y, z - 0.05);
                let b = Vec3::new(x - 0.05, y, z + 0.05);
                let c = Vec3::new(x + 0.05, y, z + 0.05);
                let d = Vec3::new(x + 0.05, y, z - 0.05);
                ContactPatch {
                    id: format!("{}-section", bond.id),
                    bond_id: bond.id.clone(),
                    a: bond.a.clone(),
                    b: bond.b.clone(),
                    normal: Vec3::Y,
                    triangles: vec![[a, b, c], [a, c, d]],
                    area: bond.area,
                    rims: Vec::new(),
                }
            })
            .collect(),
    );
    for record in manifest.chunks {
        let local = record.transform();
        let size = if record.id == BEAM {
            Vec3::new(3.0, 0.2, 0.2)
        } else {
            Vec3::new(0.1, 1.0, 0.1)
        };
        let chunk = world
            .spawn((
                Name::new(record.id.clone()),
                local,
                GlobalTransform::from(pose) * local,
                Visibility::Inherited,
                ChildOf(root),
                Collider::cuboid(size.x, size.y, size.z),
                ColliderDensity(0.0),
                Mass(record.mass),
                CenterOfMass(Vec3::ZERO),
                AngularInertia::from_mat3_unchecked(Mat3::from_cols_array_2d(&record.inertia)),
                NoAutoMass,
                NoAutoAngularInertia,
                NoAutoCenterOfMass,
                ChunkIdentity {
                    building: root,
                    id: record.id.clone(),
                },
            ))
            .id();
        let mesh = world
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::from_size(size));
        let primitive = world
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material.clone()),
                Transform::IDENTITY,
                ChildOf(chunk),
            ))
            .id();
        instance.chunks.insert(
            record.id,
            ChunkInstance {
                entity: chunk,
                primitives: vec![primitive],
                alive: true,
                triangles: Vec::new(),
                transform: local,
                material: "concrete".into(),
                detail: None,
                cluster: None,
            },
        );
    }
    world.entity_mut(root).insert(instance);
    root
}

pub(crate) fn remove_left_support(world: &mut World, root: Entity) {
    let entity = {
        let mut instance = world.get_mut::<BuildingInstance>(root).unwrap();
        let chunk = instance.chunks.get_mut(LEFT_SUPPORT).unwrap();
        chunk.alive = false;
        let entity = chunk.entity;
        instance.bonds.insert(LEFT_BOND.into(), false);
        instance.topology_dirty = true;
        entity
    };
    world
        .entity_mut(entity)
        .insert((RemovedChunk, Visibility::Hidden));
}

fn capture(commands: &mut Commands, directory: &std::path::Path, name: &str) {
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(directory.join(name)));
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive(
    mut commands: Commands,
    options: Res<DemoOptions>,
    mut state: ResMut<StressSmokeState>,
    mut settings: ResMut<StressSettings>,
    buildings: Query<(&BuildingInstance, Option<&StressState>), Without<StressProbe>>,
    probes: Query<(&BuildingInstance, &StressState), With<StressProbe>>,
    bodies: Query<(
        &Cluster,
        &Position,
        &Rotation,
        &CenterOfMass,
        &RigidBody,
        Has<Sleeping>,
    )>,
    projectiles: Query<(Entity, Has<Collider>, &Mass, &LinearVelocity), With<RigidProjectile>>,
    mut launches: ResMut<LaunchRequests>,
    impacts: Res<ImpactStats>,
    mut cameras: Query<&mut Transform, With<DemoCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(directory) = &options.stress_smoke_output else {
        return;
    };
    settings.damage_seconds = 2.0;
    settings.immediate_utilization = 100.0;
    state.waiting += 1;
    let Some((building, stress)) = buildings.iter().next() else {
        return;
    };
    if let Some(error) = &building.error {
        state.errors.push(format!("Building load failed: {error}"));
        finish(&state, &options, directory, &mut exit);
        return;
    }
    let Some(stress) = stress.filter(|stress| stress.solves > 0) else {
        if state.waiting == 1800 {
            state
                .errors
                .push("Initial selfweight solve did not complete".into());
            finish(&state, &options, directory, &mut exit);
        }
        return;
    };
    state.warmup += 1;
    if state.warmup <= 150 {
        return;
    }
    state.frame += 1;
    if state.frame < 800
        && (stress.broken_total != 0 || stress.reports.iter().any(|report| !report.valid))
    {
        state
            .errors
            .push("Intact shipped building was unstable or its stress solve failed".into());
        finish(&state, &options, directory, &mut exit);
        return;
    }
    let probe = state.probe.and_then(|root| probes.get(root).ok());
    if state.frame >= 800
        && let Some(target) = state.impact_target
    {
        for mut camera in &mut cameras {
            *camera = Transform::from_translation(target + Vec3::new(3.0, 2.0, 8.0))
                .looking_at(target, Vec3::Y);
        }
    } else if state.frame >= 25 {
        for mut camera in &mut cameras {
            *camera =
                Transform::from_xyz(33.7, 2.8, 5.5).looking_at(Vec3::new(30.0, 0.7, 0.0), Vec3::Y);
        }
    }
    state.impact_contacts = impacts.contacts;
    state.impact_broken_bonds = impacts.broken_bonds;
    state.max_impulse = impacts.max_impulse;
    if let Some((instance, stress)) = probe {
        state.broken_bonds = stress.broken_total;
        if let Some(cluster) = instance.chunks[BEAM].cluster
            && let Ok((cluster, position, rotation, center, kind, sleeping)) = bodies.get(cluster)
        {
            let height = (position.0 + rotation.0 * center.0).y;
            state.stress_released |=
                stress.broken_total > 0 && !cluster.anchored && *kind == RigidBody::Dynamic;
            state.fell |= state.stress_released && height < 0.8;
            state.slept |= state.fell && sleeping;
            if !height.is_finite() {
                state
                    .errors
                    .push("Beam developed a non-finite physical pose".into());
            }
        }
    }
    match state.frame {
        1 => {
            std::fs::create_dir_all(directory).expect("Create stress smoke output");
        }
        10 => {
            state.asset_max_utilization = stress.max_utilization;
            state.asset_reports = stress
                .reports
                .iter()
                .map(|report| {
                    serde_json::json!({
                        "chunks":report.chunks,"valid":report.valid,"iterations":report.iterations,
                        "relative_residual":report.relative_residual,"error":report.error,
                        "force_balance_error_n":report.force_balance_error,
                        "moment_balance_error_nm":report.moment_balance_error,
                        "max_displacement_m":report.max_displacement,
                    })
                })
                .collect();
            capture(&mut commands, directory, "01-intact-selfweight.png");
        }
        20 => {
            commands.queue(|world: &mut World| {
                let probe = spawn_beam(world, Vec3::new(30.0, 0.0, 0.0));
                world.resource_mut::<StressSmokeState>().probe = Some(probe);
            });
        }
        80 => {
            if let Some((_, stress)) = probe {
                state.baseline_utilization = stress.max_utilization;
                if stress.broken_total != 0
                    || stress.reports.iter().any(|report| !report.valid)
                    || stress.max_utilization >= 1.0
                {
                    state
                        .errors
                        .push("Two-support beam did not start below capacity".into());
                }
            } else {
                state.errors.push("Missing stress beam fixture".into());
            }
            capture(&mut commands, directory, "02-two-supports.png");
        }
        100 => {
            if let Some(root) = state.probe {
                commands.queue(move |world: &mut World| remove_left_support(world, root));
            }
        }
        102 => {
            if let Some((instance, stress)) = probe {
                if let Some(bond) = stress.bonds.get(RIGHT_BOND) {
                    state.overload_utilization = bond.stress.utilization;
                    state.overload_damage = bond.damage;
                    state.overload_load = Some(serde_json::json!({
                        "force_n":bond.stress.force,"moment_nm":bond.stress.moment,
                        "tension_pa":bond.stress.tension,"compression_pa":bond.stress.compression,
                        "shear_pa":bond.stress.shear,"section_exact":bond.stress.section_exact,
                    }));
                    state.saw_anchored_overload = instance.bonds[RIGHT_BOND]
                        && bond.valid
                        && bond.stress.utilization > 1.0
                        && bond.damage > 0.0
                        && bond.damage < 1.0;
                }
                if !state.saw_anchored_overload {
                    state.errors.push(
                        "Support removal did not create a still-connected progressive overload"
                            .into(),
                    );
                }
            }
            capture(&mut commands, directory, "03-overloaded-connection.png");
        }
        160 => {
            if !state.stress_released {
                state
                    .errors
                    .push("Calculated overload did not fracture and release the beam".into());
            }
            capture(&mut commands, directory, "04-stress-release.png");
        }
        760 => {
            if !state.fell || !state.slept {
                state
                    .errors
                    .push("Stress-released beam did not fall and reach physical sleep".into());
            }
            capture(&mut commands, directory, "05-settled.png");
        }
        800 | 900 => {
            let bounds = building.bounds.unwrap();
            let target_x = bounds.min.x
                + (bounds.max.x - bounds.min.x) * if state.frame == 800 { 0.3 } else { 0.7 };
            let desired = Vec3::new(
                target_x,
                bounds.min.y + (bounds.max.y - bounds.min.y) * 0.55,
                bounds.max.z,
            );
            let target = building
                .chunks
                .values()
                .filter(|chunk| {
                    chunk.alive
                        && chunk.cluster.is_some_and(|entity| {
                            bodies.get(entity).is_ok_and(|body| body.0.anchored)
                        })
                })
                .flat_map(|chunk| &chunk.triangles)
                .filter(|triangle| {
                    (triangle[1] - triangle[0])
                        .cross(triangle[2] - triangle[0])
                        .normalize_or_zero()
                        .z
                        > 0.95
                })
                .map(|triangle| (triangle[0] + triangle[1] + triangle[2]) / 3.0)
                .filter(|point| point.z >= bounds.max.z - 0.1)
                .min_by(|a, b| {
                    a.distance_squared(desired)
                        .total_cmp(&b.distance_squared(desired))
                });
            if let Some(target) = target {
                state.impact_target = Some(target);
                state.known_projectiles =
                    projectiles.iter().map(|projectile| projectile.0).collect();
                state.before_contacts = impacts.contacts;
                state.before_impact_breaks = impacts.broken_bonds;
                launches.0.push_back(LaunchRequest {
                    origin: target + Vec3::Z * 5.0,
                    direction: Dir3::NEG_Z,
                    shape: if state.frame == 800 {
                        ProjectileShape::Sphere
                    } else {
                        ProjectileShape::Cube
                    },
                });
            } else {
                state
                    .errors
                    .push("No intact front wall for rigid impact validation".into());
            }
        }
        810 | 910 => {
            let survived = projectiles
                .iter()
                .filter(|projectile| !state.known_projectiles.contains(&projectile.0))
                .any(|(_, collider, mass, velocity)| {
                    collider
                        && (mass.0 - PROJECTILE_MASS).abs() < 0.1
                        && velocity.0.is_finite()
                        && (velocity.z + PROJECTILE_SPEED).abs() > 1.0
                });
            let contacted = impacts.contacts > state.before_contacts
                && impacts.broken_bonds > state.before_impact_breaks;
            if state.frame == 810 {
                state.sphere_survived = survived;
                state.sphere_contact = contacted;
            } else {
                state.cube_survived = survived;
                state.cube_contact = contacted;
            }
            if !survived {
                state.errors.push("Rigid impactor disappeared, lost collider/mass or did not exchange normal velocity".into());
            }
            if !contacted {
                state.errors.push("Rigid impact did not produce solved contacts and collision-driven bond fracture".into());
            }
            capture(
                &mut commands,
                directory,
                if state.frame == 810 {
                    "06-rigid-sphere-impact.png"
                } else {
                    "07-rigid-cube-impact.png"
                },
            );
        }
        1000 => {
            for name in [
                "01-intact-selfweight.png",
                "02-two-supports.png",
                "03-overloaded-connection.png",
                "04-stress-release.png",
                "05-settled.png",
                "06-rigid-sphere-impact.png",
                "07-rigid-cube-impact.png",
            ] {
                if !directory.join(name).is_file() {
                    state
                        .errors
                        .push(format!("Missing stress screenshot {name}"));
                }
            }
            finish(&state, &options, directory, &mut exit);
        }
        _ => {}
    }
}

fn finish(
    state: &StressSmokeState,
    options: &DemoOptions,
    directory: &std::path::Path,
    exit: &mut MessageWriter<AppExit>,
) {
    let report = serde_json::json!({
        "asset":options.asset,"asset_stress_reports":state.asset_reports,"baseline_utilization":state.baseline_utilization,
        "asset_max_utilization":state.asset_max_utilization,"overload_load":state.overload_load,
        "sphere_survived_and_changed_velocity":state.sphere_survived,"cube_survived_and_changed_velocity":state.cube_survived,
        "sphere_contact_fracture":state.sphere_contact,"cube_contact_fracture":state.cube_contact,
        "impact_contacts":state.impact_contacts,"impact_broken_bonds":state.impact_broken_bonds,"max_impact_impulse_ns":state.max_impulse,
        "overload_utilization":state.overload_utilization,"overload_damage":state.overload_damage,
        "saw_anchored_overload":state.saw_anchored_overload,"stress_released":state.stress_released,
        "fell":state.fell,"slept":state.slept,"stress_broken_bonds":state.broken_bonds,
        "fixture":{"beam_mass_kg":288.0,"column_mass_kg":24.0,"section_area_m2":0.01,"tension_strength_pa":3e6,"damage_seconds":2.0,"immediate_utilization":100.0},
        "frames":state.frame,"errors":state.errors,
    });
    std::fs::create_dir_all(directory).expect("Create stress smoke output");
    std::fs::write(
        directory.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .expect("Write stress smoke report");
    if state.errors.is_empty() {
        info!("Stress smoke test passed: {report}");
        exit.write(AppExit::Success);
    } else {
        error!("Stress smoke test failed: {report}");
        exit.write(AppExit::error());
    }
}

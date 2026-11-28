use std::{collections::BTreeMap, fmt::Write};

use avian3d::prelude::*;
use bevy::{
    asset::AssetEventSystems, camera::visibility::VisibilitySystems,
    gizmos::config::GizmoConfigGroup, mesh::MeshTag, prelude::*, window::PrimaryWindow,
};

use crate::{
    asset::DestructibleAsset,
    camera::CameraControl,
    force_samples::{DynamicForceSample, ForceCapture, ForceSamples, bond_signature},
    force_visibility::{
        ChunkKey, ForceVisibilityCache, SurfaceHit, VisibilityChunk, VisibilityOccluder,
    },
    material::{HEAT_AMBER_TAG, HEAT_RED_TAG},
    physics::Cluster,
    scene::{
        BondInstance, BuildingInstance, ChunkIdentity, ChunkInstance, ChunkPrimitive, DemoCamera,
        chunk_primitives,
    },
    stress::BondStress,
    stress_runtime::{StressSettings, StressState, StructuralLoads},
};

const MAX_INSPECTED_BONDS: usize = 3;

#[derive(Default, Reflect, GizmoConfigGroup)]
struct ForceGizmos;

#[derive(Component)]
enum ForceHud {
    Legend,
    Inspection,
}

#[derive(Resource, Default)]
struct Aim {
    hit: Option<SurfaceHit>,
    arrow_length: f32,
    queried: bool,
}

#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
pub struct ForceViewStats {
    pub static_bonds: usize,
    pub dynamic_bonds: usize,
    pub max_force_n: f64,
    pub tinted_chunks: usize,
    pub inspected: Option<ForceInspection>,
    pub force_arrows: usize,
    pub pick_queries: usize,
}

#[derive(Reflect)]
pub struct ForceInspection {
    pub chunk: ChunkKey,
    pub bonds: Vec<ForceBondReadout>,
    pub impulse_ns: Option<f64>,
}

#[derive(Reflect)]
pub struct ForceBondReadout {
    pub id: String,
    pub force_n: f64,
    pub moment_nm: f64,
    pub utilization: f64,
    pub impact: bool,
}

pub struct ForceDebugPlugin;

impl Plugin for ForceDebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ForceCapture>()
            .init_resource::<ForceSamples>()
            .init_resource::<ForceViewStats>()
            .init_resource::<Aim>()
            .init_gizmo_group::<ForceGizmos>()
            .add_systems(Startup, setup)
            .add_systems(Update, toggle)
            .add_systems(
                PostUpdate,
                (
                    pick_target
                        .after(AssetEventSystems)
                        .after(VisibilitySystems::CheckVisibility),
                    draw,
                    update_hud,
                )
                    .chain(),
            );
    }
}

fn setup(mut commands: Commands, mut configs: ResMut<GizmoConfigStore>) {
    let (config, _) = configs.config_mut::<ForceGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 2.5;
    for (name, panel, position) in [
        (
            "Force legend",
            ForceHud::Legend,
            Node {
                left: px(18.0),
                top: px(18.0),
                ..default()
            },
        ),
        (
            "Force inspection",
            ForceHud::Inspection,
            Node {
                right: px(18.0),
                top: px(62.0),
                width: px(310.0),
                max_width: percent(55.0),
                ..default()
            },
        ),
    ] {
        commands.spawn((
            Name::new(name),
            panel,
            Text::new(""),
            TextFont {
                font_size: px(16.0).into(),
                ..default()
            },
            TextColor(Color::srgb(0.94, 0.96, 1.0)),
            BackgroundColor(Color::srgba(0.015, 0.02, 0.03, 0.88)),
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                padding: UiRect::all(px(10.0)),
                ..position
            },
            ZIndex(30),
        ));
    }
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut capture: ResMut<ForceCapture>) {
    if keys.just_pressed(KeyCode::Digit9) || keys.just_pressed(KeyCode::Numpad9) {
        capture.enabled = !capture.enabled;
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn pick_target(
    capture: Res<ForceCapture>,
    control: Res<CameraControl>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &Projection, &GlobalTransform), With<DemoCamera>>,
    instances: Query<(Entity, &BuildingInstance)>,
    poses: Query<(&GlobalTransform, &ChildOf), With<Cluster>>,
    chunks: Query<(&ChunkInstance, &ColliderOf, &InheritedVisibility)>,
    occluders: Query<
        (&Collider, &GlobalTransform, &InheritedVisibility),
        (Without<ChunkIdentity>, With<Mesh3d>),
    >,
    mut events: MessageReader<AssetEvent<DestructibleAsset>>,
    mut cache: Local<ForceVisibilityCache>,
    mut aim: ResMut<Aim>,
) {
    for event in events.read() {
        if let AssetEvent::Removed { id } | AssetEvent::Modified { id } = event {
            cache.invalidate(*id);
        }
    }
    *aim = Aim::default();
    if !capture.enabled {
        return;
    }
    let held = control.captured
        && window.single().is_ok_and(|window| window.focused)
        && (keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight));
    if !held {
        return;
    }
    let Ok((camera, Projection::Perspective(projection), camera_pose)) = cameras.single() else {
        return;
    };
    if !camera.is_active {
        return;
    }
    let Some(viewport) = camera.logical_viewport_rect() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(camera_pose, viewport.center()) else {
        return;
    };

    let mut source_chunks = Vec::new();
    for (building, instance) in &instances {
        for (id, &entity) in &instance.chunks {
            let Ok((chunk, collider, visible)) = chunks.get(entity) else {
                continue;
            };
            if !chunk.alive || !visible.get() {
                continue;
            }
            let Ok((pose, parent)) = poses.get(collider.body) else {
                continue;
            };
            if parent.parent() != building {
                continue;
            }
            source_chunks.push(VisibilityChunk {
                key: (building, id.clone()),
                asset: instance.asset.id(),
                triangles: &chunk.triangles,
                world_from_rest: pose.affine(),
                visible: true,
            });
        }
    }
    let extras = occluders
        .iter()
        .filter(|(_, _, visible)| visible.get())
        .map(|(collider, pose, _)| {
            let pose = pose.compute_transform();
            VisibilityOccluder {
                collider,
                // The collider already accounts for scale.
                world_from_local: Transform::from_translation(pose.translation)
                    .with_rotation(pose.rotation)
                    .compute_affine(),
            }
        })
        .collect::<Vec<_>>();
    aim.queried = true;
    aim.hit = cache.pick(
        ray,
        projection.far - projection.near,
        &source_chunks,
        &extras,
    );
    if let Some(hit) = &aim.hit {
        aim.arrow_length = (0.12 * hit.point.distance(ray.origin)).min(0.35);
    }
}

fn sample_members_current<'a>(
    sample: &DynamicForceSample,
    instance: &BuildingInstance,
    cluster: &Cluster,
    parent: &ChildOf,
    chunk: impl Fn(Entity) -> Option<(&'a ChunkInstance, Option<&'a ColliderOf>)>,
    current_bond_signature: impl FnOnce() -> u64,
) -> bool {
    parent.parent() == sample.building
        && !cluster.anchored
        && cluster.chunks.len() == sample.chunks.len()
        && sample.chunks.iter().all(|id| {
            instance
                .chunks
                .get(id)
                .and_then(|&entity| chunk(entity))
                .is_some_and(|(chunk, collider)| {
                    chunk.alive && collider.is_some_and(|collider| collider.body == sample.body)
                })
        })
        && sample.bond_signature == current_bond_signature()
}

fn heat_tag(utilization: f64) -> u32 {
    if utilization >= 1.0 {
        HEAT_RED_TAG
    } else if utilization >= 0.5 {
        HEAT_AMBER_TAG
    } else {
        0
    }
}

fn direction_on_chunk(pose: &GlobalTransform, force: [f64; 3], is_a: bool) -> Option<Vec3> {
    let direction = bevy::math::DVec3::from_array(force)
        .try_normalize()?
        .as_vec3();
    // The restoring force is +traction on a and -traction on b.
    let direction = if is_a { direction } else { -direction };
    Some(pose.rotation() * direction)
}

struct InspectionBond {
    readout: ForceBondReadout,
    direction: Option<Vec3>,
}

fn order_inspection_bonds(bonds: &mut Vec<InspectionBond>) {
    bonds.sort_by(|a, b| {
        b.readout
            .utilization
            .total_cmp(&a.readout.utilization)
            .then_with(|| a.readout.id.cmp(&b.readout.id))
    });
    bonds.truncate(MAX_INSPECTED_BONDS);
}

fn update_heat_tags(
    desired: BTreeMap<Entity, u32>,
    painted: &mut BTreeMap<Entity, u32>,
    tags: &mut Query<&mut MeshTag, With<ChunkPrimitive>>,
) {
    for &entity in painted
        .keys()
        .filter(|entity| !desired.contains_key(entity))
    {
        if let Ok(mut tag) = tags.get_mut(entity) {
            tag.set_if_neq(MeshTag::new(0));
        }
    }
    for (&entity, &value) in &desired {
        if painted.get(&entity) != Some(&value)
            && let Ok(mut tag) = tags.get_mut(entity)
        {
            tag.set_if_neq(MeshTag::new(value));
        }
    }
    *painted = desired;
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    capture: Res<ForceCapture>,
    aim: Res<Aim>,
    mut samples: ResMut<ForceSamples>,
    mut stats: ResMut<ForceViewStats>,
    time: Res<Time<Fixed>>,
    gravity: Res<Gravity>,
    settings: Res<StressSettings>,
    instances: Query<(
        Entity,
        Ref<BuildingInstance>,
        Option<&StressState>,
        Option<Ref<StructuralLoads>>,
        &GlobalTransform,
    )>,
    chunks: Query<(&ChunkIdentity, &ChunkInstance, Option<&ColliderOf>)>,
    children: Query<&Children>,
    bonds: Query<&BondInstance>,
    bodies: Query<(&GlobalTransform, &RigidBody, &Cluster, &ChildOf)>,
    mut tags: Query<&mut MeshTag, With<ChunkPrimitive>>,
    mut painted: Local<BTreeMap<Entity, u32>>,
    mut gizmos: Gizmos<ForceGizmos>,
) {
    *stats = ForceViewStats::default();
    if !capture.enabled {
        samples.clear();
        update_heat_tags(BTreeMap::new(), &mut painted, &mut tags);
        return;
    }
    stats.pick_queries = usize::from(aim.queried);
    let now = time.elapsed_secs_f64();
    let mut signatures = BTreeMap::new();
    samples.prune(
        now,
        |sample| {
            instances
                .get(sample.building)
                .ok()
                .zip(bodies.get(sample.body).ok())
                .is_some_and(|((_, instance, _, _, _), (_, kind, cluster, parent))| {
                    *kind == RigidBody::Dynamic
                        && sample_members_current(
                            sample,
                            &instance,
                            cluster,
                            parent,
                            |entity| {
                                chunks
                                    .get(entity)
                                    .ok()
                                    .map(|(_, chunk, collider)| (chunk, collider))
                            },
                            || {
                                *signatures.entry(sample.building).or_insert_with(|| {
                                    bond_signature(instance.bonds.iter().map(|(id, &entity)| {
                                        (
                                            id.as_str(),
                                            bonds.get(entity).is_ok_and(|bond| bond.active),
                                        )
                                    }))
                                })
                            },
                        )
                })
        },
        |sample| {
            instances
                .get(sample.building)
                .is_ok_and(|(_, instance, _, _, _)| {
                    instance
                        .chunks
                        .get(&sample.chunk)
                        .and_then(|&entity| chunks.get(entity).ok())
                        .is_some_and(|(_, chunk, _)| chunk.alive)
                })
        },
    );

    let mut desired = BTreeMap::new();
    let mut inspection_bonds = Vec::new();
    for (building, instance, state, loads, root) in &instances {
        let selected = aim.hit.as_ref().filter(|hit| hit.key.0 == building);
        let current_static = state.filter(|state| {
            settings.enabled
                && state.inputs_current(
                    instance.last_changed().get(),
                    loads.as_ref().map(|loads| loads.last_changed()),
                    settings.last_changed().get(),
                    root.compute_transform().rotation.conjugate() * gravity.0,
                )
        });
        let mut heat = BTreeMap::<Entity, u32>::new();
        for &entity in instance.bonds.values() {
            let Ok(bond) = bonds.get(entity) else {
                continue;
            };
            if !bond.active {
                continue;
            }
            let Ok([(a_id, a, Some(a_collider)), (b_id, b, Some(b_collider))]) =
                chunks.get_many(bond.endpoints)
            else {
                continue;
            };
            let a_id = a_id.id.as_str();
            let b_id = b_id.id.as_str();
            if !a.alive || !b.alive || a_collider.body != b_collider.body {
                continue;
            }
            let body = a_collider.body;
            let Ok((pose, kind, _, parent)) = bodies.get(body) else {
                continue;
            };
            if parent.parent() != building {
                continue;
            }
            let impact = *kind == RigidBody::Dynamic;
            let stress = match kind {
                RigidBody::Static => current_static
                    .and_then(|state| state.bonds.get(&bond.id))
                    .filter(|load| load.valid)
                    .map(|load| &load.stress),
                RigidBody::Dynamic => samples
                    .dynamic
                    .iter()
                    .find(|sample| sample.building == building && sample.body == body)
                    .and_then(|sample| sample.bonds.get(&bond.id)),
                _ => None,
            };
            let Some(stress) = stress else {
                continue;
            };
            let Some((force_n, moment_nm)) = finite_load(stress) else {
                continue;
            };

            if impact {
                stats.dynamic_bonds += 1;
            } else {
                stats.static_bonds += 1;
            }
            stats.max_force_n = stats.max_force_n.max(force_n);

            let tag = heat_tag(stress.utilization);
            if tag != 0 {
                for endpoint in bond.endpoints {
                    heat.entry(endpoint)
                        .and_modify(|old| *old = (*old).max(tag))
                        .or_insert(tag);
                }
            }
            if let Some(hit) = selected
                && (hit.key.1 == a_id || hit.key.1 == b_id)
            {
                inspection_bonds.push(InspectionBond {
                    direction: direction_on_chunk(pose, stress.force, hit.key.1 == a_id),
                    readout: ForceBondReadout {
                        id: bond.id.clone(),
                        force_n,
                        moment_nm,
                        utilization: stress.utilization,
                        impact,
                    },
                });
            }
        }
        stats.tinted_chunks += heat.len();
        for (entity, tag) in heat {
            desired.extend(
                chunk_primitives(children.get(entity).ok(), |entity| tags.contains(entity))
                    .map(|primitive| (primitive, tag)),
            );
        }
    }
    update_heat_tags(desired, &mut painted, &mut tags);

    if let Some(hit) = &aim.hit {
        order_inspection_bonds(&mut inspection_bonds);
        if let Some(direction) = inspection_bonds.first().and_then(|bond| bond.direction)
            && aim.arrow_length > 0.0001
        {
            gizmos.arrow(
                hit.point,
                hit.point + direction * aim.arrow_length,
                Color::srgb(0.24, 0.90, 1.0),
            );
            stats.force_arrows = 1;
        }
        let impulse_ns = samples
            .impulses
            .iter()
            .filter(|sample| sample.building == hit.key.0 && sample.chunk == hit.key.1)
            .map(|sample| sample.impulse.as_dvec3().length())
            .filter(|value| value.is_finite() && *value > 0.0)
            .max_by(f64::total_cmp);
        stats.inspected = Some(ForceInspection {
            chunk: hit.key.clone(),
            bonds: inspection_bonds
                .into_iter()
                .map(|bond| bond.readout)
                .collect(),
            impulse_ns,
        });
    }
}

fn finite_load(stress: &BondStress) -> Option<(f64, f64)> {
    let force_n = bevy::math::DVec3::from_array(stress.force).length();
    let moment_nm = bevy::math::DVec3::from_array(stress.moment).length();
    (force_n.is_finite() && moment_nm.is_finite() && stress.utilization.is_finite())
        .then_some((force_n, moment_nm))
}

fn quantity(value: f64, unit: &str) -> String {
    if value >= 1e6 {
        format!("{:.2} M{unit}", value / 1e6)
    } else if value >= 1e3 {
        format!("{:.2} k{unit}", value / 1e3)
    } else {
        format!("{value:.2} {unit}")
    }
}

fn inspection_text(inspection: &ForceInspection) -> String {
    let mut text = format!("Chunk {}", inspection.chunk.1);
    if inspection.bonds.is_empty() {
        text.push_str("\nNo valid connection loads");
    }
    for (index, bond) in inspection.bonds.iter().enumerate() {
        let label = if bond.impact {
            "Impact J/10 ms"
        } else {
            "Static"
        };
        let _ = write!(
            text,
            "\n{}  U {:.1}%  {}\nF {} | M {}",
            index + 1,
            100.0 * bond.utilization,
            label,
            quantity(bond.force_n, "N"),
            quantity(bond.moment_nm, "N*m"),
        );
    }
    if let Some(impulse) = inspection.impulse_ns {
        let _ = write!(text, "\nContact J {}", quantity(impulse, "N*s"));
    }
    text
}

fn update_hud(
    capture: Res<ForceCapture>,
    stats: Res<ForceViewStats>,
    mut hud: Query<(&ForceHud, &mut Text, &mut Node)>,
) {
    for (panel, mut text, mut node) in &mut hud {
        let content = match panel {
            ForceHud::Legend if capture.enabled => {
                Some("9: LOAD HEAT | Amber 50% to <100% | Red >=100% | Hold Alt to inspect".into())
            }
            ForceHud::Inspection => stats.inspected.as_ref().map(inspection_text),
            _ => None,
        };
        let display = if content.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        if let Some(content) = content
            && text.0 != content
        {
            text.0 = content;
        }
    }
}

#[cfg(test)]
mod tests;

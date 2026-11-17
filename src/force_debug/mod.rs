use std::collections::{BTreeMap, BTreeSet};

use avian3d::prelude::*;
use bevy::{
    camera::{CameraProjection, visibility::VisibilitySystems},
    gizmos::config::GizmoConfigGroup,
    prelude::*,
};

use crate::{
    asset::DestructibleAsset,
    force_samples::{
        DynamicForceSample, FORCE_SAMPLE_SECONDS, ForceCapture, ForceSamples,
        IMPULSE_SAMPLE_SECONDS, bond_signature,
    },
    force_visibility::{
        ForceVisibilityCache, VisibilityCamera, VisibilityChunk, VisibilityOccluder,
    },
    physics::Cluster,
    scene::{BuildingInstance, ChunkIdentity, DemoCamera},
    stress::BondStress,
    stress_runtime::{StressSettings, StressState, StructuralLoads},
};

const ARROW_LENGTH: f32 = 0.75;
const MAX_FORCE_GLYPHS: usize = 400;
const MAX_GRAPH_BONDS: usize = 3000;

#[derive(Default, Reflect, GizmoConfigGroup)]
struct ForceGizmos;

#[derive(Component)]
struct ForceHud;

#[derive(Resource, Default, Clone, Debug)]
pub struct ForceViewStats {
    pub graph_nodes: usize,
    pub graph_bonds: usize,
    pub static_bonds: usize,
    pub dynamic_bonds: usize,
    pub force_pairs: usize,
    pub force_arrows: usize,
    pub impulses: usize,
    pub stale_components: usize,
    pub max_force_n: f64,
    pub max_moment_nm: f64,
    pub max_impulse_ns: f64,
    pub max_utilization: f64,
    pub drawn_bonds: Vec<(Entity, String)>,
    pub drawn_graph_bonds: Vec<(Entity, String)>,
    pub visible_chunks: Vec<(Entity, String)>,
    pub visibility_rays: usize,
    pub visibility_untested: usize,
}

pub struct ForceDebugPlugin;

impl Plugin for ForceDebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ForceCapture>()
            .init_resource::<ForceSamples>()
            .init_resource::<ForceViewStats>()
            .init_resource::<ForceVisibilityCache>()
            .init_gizmo_group::<ForceGizmos>()
            .add_systems(Startup, setup)
            .add_systems(Update, (toggle, invalidate_visibility_cache))
            .add_systems(
                PostUpdate,
                (
                    draw.after(VisibilitySystems::CheckVisibility),
                    update_hud.after(draw),
                ),
            );
    }
}

fn invalidate_visibility_cache(
    mut events: MessageReader<AssetEvent<DestructibleAsset>>,
    mut cache: ResMut<ForceVisibilityCache>,
) {
    for event in events.read() {
        if let AssetEvent::Removed { id } | AssetEvent::Modified { id } = event {
            cache.invalidate(*id);
        }
    }
}

fn setup(mut commands: Commands, mut configs: ResMut<GizmoConfigStore>) {
    let (config, _) = configs.config_mut::<ForceGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 1.5;
    commands.spawn((
        Name::new("Structural force legend"),
        ForceHud,
        Text::new(""),
        TextFont {
            font_size: px(16.0).into(),
            ..default()
        },
        TextColor(Color::srgb(0.94, 0.96, 1.0)),
        BackgroundColor(Color::srgba(0.015, 0.02, 0.03, 0.88)),
        Node {
            position_type: PositionType::Absolute,
            left: px(18.0),
            top: px(18.0),
            padding: UiRect::all(px(10.0)),
            ..default()
        },
        Visibility::Hidden,
        ZIndex(30),
    ));
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut capture: ResMut<ForceCapture>) {
    if keys.just_pressed(KeyCode::Digit9) || keys.just_pressed(KeyCode::Numpad9) {
        capture.enabled = !capture.enabled;
    }
}

pub(crate) fn sample_members_current(
    sample: &DynamicForceSample,
    instance: &BuildingInstance,
    cluster: &Cluster,
) -> bool {
    cluster.building == sample.building
        && !cluster.anchored
        && sample.bond_signature == bond_signature(&instance.bonds)
        && cluster.chunks.len() == sample.chunks.len()
        && sample.chunks.iter().all(|id| {
            instance
                .chunks
                .get(id)
                .is_some_and(|chunk| chunk.alive && chunk.cluster == Some(sample.body))
        })
}

pub(crate) fn scaled_length(magnitude: f64, reference: f64) -> f32 {
    if !magnitude.is_finite() || !reference.is_finite() || magnitude <= 0.0 || reference <= 0.0 {
        return 0.0;
    }
    ARROW_LENGTH * (magnitude / reference).clamp(0.0, 1.0) as f32
}

pub(crate) fn world_force_pair(pose: Transform, force: [f64; 3]) -> Option<(Vec3, Vec3, f64)> {
    let force = bevy::math::DVec3::from_array(force);
    let magnitude = force.length();
    if !magnitude.is_finite() || magnitude <= 0.0 {
        return None;
    }
    let direction = pose.rotation * (force / magnitude).as_vec3();
    Some((direction, -direction, magnitude))
}

fn utilization_color(utilization: f64, alpha: f32) -> Color {
    let (r, g, b) = if utilization >= 1.0 {
        (1.0, 0.12, 0.10)
    } else if utilization >= 0.5 {
        (1.0, 0.78, 0.10)
    } else {
        (0.16, 0.95, 0.40)
    };
    Color::srgba(r, g, b, alpha)
}

struct Glyph {
    id: (Entity, String),
    center: Vec3,
    normal: Vec3,
    pose: Transform,
    force: [f64; 3],
    force_n: f64,
    moment_nm: f64,
    utilization: f64,
    age: Option<f64>,
    visible_ends: [bool; 2],
}

fn glyph(
    id: (Entity, String),
    pose: Transform,
    center: [f32; 3],
    normal: [f32; 3],
    stress: &BondStress,
    age: Option<f64>,
    visible_ends: [bool; 2],
) -> Option<Glyph> {
    let force_n = bevy::math::DVec3::from_array(stress.force).length();
    let moment_nm = bevy::math::DVec3::from_array(stress.moment).length();
    (force_n.is_finite() && moment_nm.is_finite() && stress.utilization.is_finite()).then_some(
        Glyph {
            id,
            center: pose.transform_point(center.into()),
            normal: pose.rotation * Vec3::from_array(normal),
            pose,
            force: stress.force,
            force_n,
            moment_nm,
            utilization: stress.utilization,
            age,
            visible_ends,
        },
    )
}

fn diamond(
    gizmos: &mut Gizmos<ForceGizmos>,
    center: Vec3,
    normal: Vec3,
    radius: f32,
    color: Color,
) {
    let normal = normal.normalize_or(Vec3::Y);
    let axis = if normal.x.abs() < 0.8 {
        Vec3::X
    } else {
        Vec3::Y
    };
    let tangent = normal.cross(axis).normalize();
    let bitangent = normal.cross(tangent);
    let ring = [tangent, bitangent, -tangent, -bitangent].map(|point| center + point * radius);
    for index in 0..4 {
        gizmos.line(ring[index], ring[(index + 1) % 4], color);
        gizmos.line(center + normal * radius, ring[index], color);
        gizmos.line(center - normal * radius, ring[index], color);
    }
}

fn arrow(
    gizmos: &mut Gizmos<ForceGizmos>,
    origin: Vec3,
    direction: Vec3,
    length: f32,
    color: Color,
) {
    if length <= 0.0001 {
        return;
    }
    let end = origin + direction * length;
    let axis = if direction.y.abs() < 0.8 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let side = direction.cross(axis).normalize();
    let tip = (length * 0.22).min(0.12);
    gizmos.line(origin, end, color);
    gizmos.line(end, end - direction * tip + side * tip * 0.45, color);
    gizmos.line(end, end - direction * tip - side * tip * 0.45, color);
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    capture: Res<ForceCapture>,
    mut samples: ResMut<ForceSamples>,
    mut stats: ResMut<ForceViewStats>,
    time: Res<Time<Fixed>>,
    assets: Res<Assets<DestructibleAsset>>,
    gravity: Res<Gravity>,
    settings: Res<StressSettings>,
    loads: Res<StructuralLoads>,
    instances: Query<(
        Entity,
        Ref<BuildingInstance>,
        Option<&StressState>,
        &GlobalTransform,
    )>,
    bodies: Query<(&GlobalTransform, &RigidBody, &Cluster)>,
    cameras: Query<(&Camera, &Projection, &GlobalTransform), With<DemoCamera>>,
    inherited: Query<&InheritedVisibility>,
    occluders: Query<
        (
            Entity,
            Ref<Collider>,
            &GlobalTransform,
            &InheritedVisibility,
        ),
        (Without<ChunkIdentity>, With<Mesh3d>),
    >,
    mut visibility_cache: ResMut<ForceVisibilityCache>,
    mut gizmos: Gizmos<ForceGizmos>,
) {
    *stats = ForceViewStats::default();
    if !capture.enabled {
        samples.clear();
        return;
    }
    let now = time.elapsed_secs_f64();
    samples.prune(
        now,
        |sample| {
            instances
                .get(sample.building)
                .ok()
                .zip(bodies.get(sample.body).ok())
                .is_some_and(|((_, instance, _, _), (_, body, cluster))| {
                    *body == RigidBody::Dynamic
                        && sample_members_current(sample, &instance, cluster)
                })
        },
        |sample| {
            instances
                .get(sample.building)
                .is_ok_and(|(_, instance, _, _)| {
                    instance
                        .chunks
                        .get(&sample.chunk)
                        .is_some_and(|chunk| chunk.alive)
                })
        },
    );
    let Ok((camera, Projection::Perspective(projection), camera_pose)) = cameras.single() else {
        return;
    };
    if !camera.is_active {
        return;
    }
    let view = camera_pose.affine().inverse();
    let camera = VisibilityCamera {
        clip_from_world: projection.get_clip_from_view() * camera_pose.to_matrix().inverse(),
        view_from_world: view,
        near: projection.near,
        far: projection.far,
    };
    let mut source_chunks = Vec::new();
    for (building, instance, _, _) in &instances {
        let instance = instance.into_inner();
        for (id, chunk) in &instance.chunks {
            if !chunk.alive {
                continue;
            }
            let Some((pose, _, _)) = chunk.cluster.and_then(|body| bodies.get(body).ok()) else {
                continue;
            };
            source_chunks.push(VisibilityChunk {
                key: (building, id.clone()),
                asset: instance.asset.id(),
                triangles: &chunk.triangles,
                world_from_rest: pose.affine(),
                visible: inherited
                    .get(chunk.entity)
                    .is_ok_and(|visible| visible.get()),
            });
        }
    }
    let extras = occluders
        .iter()
        .filter(|(_, _, _, visible)| visible.get())
        .map(|(entity, collider, pose, _)| {
            let revision = u64::from(collider.last_changed().get());
            let pose = pose.compute_transform();
            VisibilityOccluder {
                entity,
                collider: collider.into_inner(),
                world_from_local: Transform::from_translation(pose.translation)
                    .with_rotation(pose.rotation)
                    .compute_affine(),
                revision,
            }
        })
        .collect::<Vec<_>>();
    let visible = visibility_cache.update(true, &camera, &source_chunks, &extras);
    stats.visible_chunks = visible.visible.iter().cloned().collect();
    stats.visibility_rays = visible.stats.rays;
    stats.visibility_untested = visible.stats.pending_chunks;
    let mut glyphs = Vec::new();
    for (building, instance, state, root) in &instances {
        let Some(asset) = assets.get(&instance.asset) else {
            continue;
        };
        let visible_ids = visible
            .visible
            .iter()
            .filter(|(owner, _)| *owner == building)
            .map(|(_, id)| id.as_str())
            .collect::<BTreeSet<_>>();
        let centers = asset
            .manifest
            .chunks
            .iter()
            .map(|record| {
                (
                    record.id.as_str(),
                    record
                        .transform()
                        .transform_point(record.center_of_mass.into()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let current_static = settings.enabled
            && state.is_some_and(|state| {
                state.inputs_current(
                    instance.last_changed().get(),
                    loads.last_changed().get(),
                    settings.last_changed().get(),
                    root.compute_transform().rotation.conjugate() * gravity.0,
                )
            });
        if !current_static {
            stats.stale_components += state.map_or(1, |state| state.reports.len().max(1));
        } else if let Some(state) = state {
            stats.stale_components += state.reports.iter().filter(|report| !report.valid).count();
        }
        for (id, chunk) in &instance.chunks {
            if !chunk.alive || !visible_ids.contains(id.as_str()) {
                continue;
            }
            let Some((transform, _, _)) = chunk.cluster.and_then(|body| bodies.get(body).ok())
            else {
                continue;
            };
            let Some(center) = centers.get(id.as_str()) else {
                continue;
            };
            let center = transform.transform_point(*center);
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                gizmos.line(
                    center - axis * 0.035,
                    center + axis * 0.035,
                    Color::srgba(0.48, 0.68, 1.0, 0.65),
                );
            }
            stats.graph_nodes += 1;
        }
        for bond in &asset.manifest.bonds {
            if !instance.bonds.get(&bond.id).copied().unwrap_or(false) {
                continue;
            }
            let visible_ends = [
                visible_ids.contains(bond.a.as_str()),
                visible_ids.contains(bond.b.as_str()),
            ];
            if !visible_ends.iter().any(|&visible| visible) {
                continue;
            }
            let Some(a) = instance.chunks.get(&bond.a).filter(|chunk| chunk.alive) else {
                continue;
            };
            let Some(b) = instance.chunks.get(&bond.b).filter(|chunk| chunk.alive) else {
                continue;
            };
            let Some(body) = a.cluster.filter(|body| Some(*body) == b.cluster) else {
                continue;
            };
            let Ok((transform, kind, _)) = bodies.get(body) else {
                continue;
            };
            let pose = transform.compute_transform();
            if stats.graph_bonds < MAX_GRAPH_BONDS
                && let Some((a, b)) = centers
                    .get(bond.a.as_str())
                    .zip(centers.get(bond.b.as_str()))
            {
                let center = pose.transform_point(bond.center.into());
                if visible_ends[0] {
                    gizmos.line(
                        pose.transform_point(*a),
                        center,
                        Color::srgba(0.86, 0.90, 0.96, 0.28),
                    );
                }
                if visible_ends[1] {
                    gizmos.line(
                        center,
                        pose.transform_point(*b),
                        Color::srgba(0.86, 0.90, 0.96, 0.28),
                    );
                }
                stats.graph_bonds += 1;
                stats.drawn_graph_bonds.push((building, bond.id.clone()));
            }
            let value = if *kind == RigidBody::Static && current_static {
                state
                    .and_then(|state| state.bonds.get(&bond.id))
                    .filter(|load| load.valid)
                    .map(|load| (&load.stress, None))
            } else if *kind == RigidBody::Dynamic {
                samples
                    .dynamic
                    .iter()
                    .find(|sample| sample.building == building && sample.body == body)
                    .and_then(|sample| {
                        sample
                            .bonds
                            .get(&bond.id)
                            .map(|stress| (stress, Some(now - sample.captured_at)))
                    })
            } else {
                None
            };
            if let Some((stress, age)) = value
                && let Some(glyph) = glyph(
                    (building, bond.id.clone()),
                    pose,
                    bond.center,
                    bond.normal,
                    stress,
                    age,
                    visible_ends,
                )
            {
                if age.is_some() {
                    stats.dynamic_bonds += 1;
                } else {
                    stats.static_bonds += 1;
                }
                glyphs.push(glyph);
            }
        }
    }
    for glyph in &glyphs {
        stats.max_force_n = stats.max_force_n.max(glyph.force_n);
        stats.max_moment_nm = stats.max_moment_nm.max(glyph.moment_nm);
        stats.max_utilization = stats.max_utilization.max(glyph.utilization);
    }
    glyphs.sort_by(|a, b| {
        b.utilization
            .total_cmp(&a.utilization)
            .then(b.force_n.total_cmp(&a.force_n))
    });
    for glyph in glyphs.iter().take(MAX_FORCE_GLYPHS) {
        stats.drawn_bonds.push(glyph.id.clone());
        let alpha = glyph
            .age
            .map_or(0.9, |age| (1.0 - age / FORCE_SAMPLE_SECONDS) as f32);
        let outline = if glyph.age.is_some() {
            Color::srgba(0.20, 0.85, 1.0, alpha)
        } else {
            Color::srgba(0.95, 0.97, 1.0, 0.65)
        };
        diamond(&mut gizmos, glyph.center, glyph.normal, 0.105, outline);
        let color = utilization_color(glyph.utilization, alpha);
        let inner = 0.092 * (glyph.utilization.clamp(0.0, 1.0).sqrt() as f32);
        if inner > 0.001 {
            diamond(&mut gizmos, glyph.center, glyph.normal, inner, color);
        }
        if let Some((a, b, magnitude)) = world_force_pair(glyph.pose, glyph.force) {
            let length = scaled_length(magnitude, stats.max_force_n);
            if glyph.visible_ends[0] {
                arrow(
                    &mut gizmos,
                    glyph.center - glyph.normal * 0.025,
                    a,
                    length,
                    color,
                );
            }
            if glyph.visible_ends[1] {
                arrow(
                    &mut gizmos,
                    glyph.center + glyph.normal * 0.025,
                    b,
                    length,
                    color,
                );
            }
            if length > 0.0001 {
                stats.force_arrows += glyph
                    .visible_ends
                    .into_iter()
                    .filter(|&visible| visible)
                    .count();
                stats.force_pairs += usize::from(glyph.visible_ends == [true, true]);
            }
        }
    }
    stats.max_impulse_ns = samples
        .impulses
        .iter()
        .filter(|sample| {
            visible
                .visible
                .contains(&(sample.building, sample.chunk.clone()))
        })
        .map(|sample| sample.impulse.as_dvec3().length())
        .filter(|value| value.is_finite())
        .fold(0.0, f64::max);
    for sample in &samples.impulses {
        if !visible
            .visible
            .contains(&(sample.building, sample.chunk.clone()))
        {
            continue;
        }
        let magnitude = sample.impulse.as_dvec3().length();
        if magnitude > 0.0 && magnitude.is_finite() {
            let alpha = (1.0 - (now - sample.captured_at) / IMPULSE_SAMPLE_SECONDS) as f32;
            arrow(
                &mut gizmos,
                sample.point,
                sample.impulse.normalize(),
                scaled_length(magnitude, stats.max_impulse_ns),
                Color::srgba(0.85, 0.35, 1.0, alpha),
            );
            stats.impulses += 1;
        }
    }
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

fn update_hud(
    capture: Res<ForceCapture>,
    stats: Res<ForceViewStats>,
    mut hud: Query<(&mut Text, &mut Visibility), With<ForceHud>>,
) {
    for (mut text, mut visibility) in &mut hud {
        let desired = if capture.enabled {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *visibility != desired {
            *visibility = desired;
        }
        if !capture.enabled {
            continue;
        }
        let available = stats.static_bonds + stats.dynamic_bonds > 0;
        let force = if available {
            quantity(stats.max_force_n, "N")
        } else {
            "unavailable (N)".into()
        };
        let moment = if available {
            quantity(stats.max_moment_nm, "N*m")
        } else {
            "unavailable (N*m)".into()
        };
        let utilization = if available {
            format!("{:.3}", stats.max_utilization)
        } else {
            "unavailable".into()
        };
        let updated = format!(
            "9: VISIBLE CHUNK FORCES | {} static + {} impact bonds | {} arrows ({} pairs)\nWhite: connection | cyan: impact equivalent J/10ms <=0.7s | purple: normal contact J <=1s\nU = stress/strength: green <0.5, yellow <1, red >=1 | visible ends of equal/opposite forces\nLinear F scale: 0.75 m = {} | separate J scale: 0.75 m = {}\nPeak visible F: {} | M: {} | U: {} | {} stale/unavailable components\nVisible chunks: {} | visibility rays: {} | conservatively hidden pending: {}",
            stats.static_bonds,
            stats.dynamic_bonds,
            stats.force_arrows,
            stats.force_pairs,
            force,
            quantity(stats.max_impulse_ns, "N*s"),
            force,
            moment,
            utilization,
            stats.stale_components,
            stats.visible_chunks.len(),
            stats.visibility_rays,
            stats.visibility_untested,
        );
        if text.0 != updated {
            text.0 = updated;
        }
    }
}

#[cfg(test)]
mod tests;

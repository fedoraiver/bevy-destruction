use std::collections::{BTreeMap, BTreeSet, VecDeque};

use avian3d::{collision::contact_types::ContactGraph, prelude::*};
use bevy::{
    math::{DMat3, DVec3},
    prelude::*,
};

use crate::{
    DemoOptions,
    asset::{DestructibleAsset, Manifest},
    camera::CameraControl,
    force_samples::{
        DynamicForceSample, ForceCapture, ForceSamples, ImpulseSample, MAX_IMPULSE_SAMPLES,
        bond_signature,
    },
    physics::Cluster,
    scene::{BuildingInstance, ChunkIdentity, DemoCamera},
    stress::{self, NodeLoad, SolverSettings},
};

#[cfg(test)]
mod tests;

pub struct ProjectilePlugin;

impl Plugin for ProjectilePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LaunchRequests>()
            .init_resource::<ImpactStats>()
            .init_resource::<ImpactState>()
            .add_message::<crate::vfx::BreakBurst>()
            .add_systems(Update, fire.after(crate::camera::move_camera))
            .add_systems(FixedUpdate, (spawn_projectiles, retire_projectiles))
            .add_systems(FixedPostUpdate, impact_damage.after(PhysicsSystems::Last));
    }
}

pub const PROJECTILE_MASS: f32 = 8000.0;
pub const PROJECTILE_SPEED: f32 = 55.0;
const IMPACT_DURATION: f32 = 0.01;
const IMPACT_MIN_SPEED: f32 = 2.0;
const IMPACT_BREAK_LIMIT: usize = 8;
// 为同一步的局部接触保留一半断裂预算。
const DYNAMIC_IMPACT_BREAK_LIMIT: usize = IMPACT_BREAK_LIMIT / 2;

#[derive(Clone, Copy)]
pub enum ProjectileShape {
    Sphere,
    Cube,
}

pub struct LaunchRequest {
    pub origin: Vec3,
    pub direction: Dir3,
    pub shape: ProjectileShape,
}

#[derive(Resource, Default)]
pub struct LaunchRequests {
    pub queue: VecDeque<LaunchRequest>,
}

#[derive(Component)]
pub struct RigidProjectile {
    age: f32,
}

#[derive(Resource, Default, Clone)]
pub struct ImpactStats {
    pub launched: u64,
    pub contacts: u64,
    pub broken_bonds: u64,
    pub last_impulse: f32,
    pub max_impulse: f32,
    pub structural_solves: u64,
    pub structural_failures: u64,
    pub pending_structural: usize,
    pub max_structural_solve_ms: f64,
    pub max_force_balance_ratio: f64,
    pub max_moment_balance_ratio: f64,
    pub max_gauge_reaction_ratio: f64,
    pub structural_broken_bonds: u64,
    pub released_structural_events: u64,
    pub rebound_fresh_structural_events: u64,
    pub invalidated_structural_events: u64,
    pub max_structural_utilization: f64,
}

#[derive(Clone, Copy, Default)]
struct ImpulseLoad {
    linear: DVec3,
    angular: DVec3,
}

struct ClusterImpact {
    building: Entity,
    chunks: Vec<String>,
    loads: BTreeMap<String, ImpulseLoad>,
    prepared: Option<InertialLoadCase>,
}

#[derive(Resource, Default)]
struct ImpactState {
    touching: BTreeSet<(Entity, Entity)>,
    pending: BTreeMap<(Entity, String), f32>,
    shocks: VecDeque<ClusterImpact>,
    released: u64,
    rebound_fresh: u64,
    invalidated: u64,
}

pub(crate) fn reset(world: &mut World) {
    world.resource_mut::<LaunchRequests>().queue.clear();
    *world.resource_mut::<ImpactState>() = ImpactState::default();
    *world.resource_mut::<ImpactStats>() = ImpactStats::default();
}

fn fire(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    options: Res<DemoOptions>,
    control: Res<CameraControl>,
    windows: Query<&Window>,
    cameras: Query<&GlobalTransform, With<DemoCamera>>,
    mut requests: ResMut<LaunchRequests>,
) {
    if options.scripted()
        || !control.captured
        || control.suppress_click
        || !windows.iter().any(|window| window.focused)
    {
        return;
    }
    let shape = if buttons.just_pressed(MouseButton::Right) {
        ProjectileShape::Sphere
    } else if keys.just_pressed(KeyCode::KeyR) {
        ProjectileShape::Cube
    } else {
        return;
    };
    if let Ok(camera) = cameras.single() {
        requests.queue.push_back(LaunchRequest {
            origin: camera.translation() + camera.forward() * 1.5,
            direction: camera.forward(),
            shape,
        });
    }
}

fn spawn_projectiles(
    mut commands: Commands,
    mut requests: ResMut<LaunchRequests>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    projectiles: Query<Entity, With<RigidProjectile>>,
    mut stats: ResMut<ImpactStats>,
) {
    let mut count = projectiles.iter().len();
    for request in requests.queue.drain(..) {
        if count >= 12 {
            continue;
        }
        let (mesh, collider) = match request.shape {
            ProjectileShape::Sphere => (Mesh::from(Sphere::new(0.65)), Collider::sphere(0.65)),
            ProjectileShape::Cube => (
                Mesh::from(Cuboid::from_length(1.2)),
                Collider::cuboid(1.2, 1.2, 1.2),
            ),
        };
        let density = ColliderDensity(PROJECTILE_MASS / collider.mass(1.0));
        commands
            .spawn((
                Name::new("Unbreakable rigid impactor"),
                RigidProjectile { age: 0.0 },
                RigidBody::Dynamic,
                collider,
                density,
                Transform::from_translation(request.origin),
                LinearVelocity(*request.direction * PROJECTILE_SPEED),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(0.9, 0.24, 0.06),
                    metallic: 0.75,
                    perceptual_roughness: 0.25,
                    ..default()
                })),
            ))
            .insert((
                Friction::new(0.4),
                Restitution::new(0.1),
                TransformInterpolation,
                SweptCcd {
                    filter: CcdFilter::ALL,
                    ..default()
                },
            ));
        count += 1;
        stats.launched += 1;
    }
}

fn retire_projectiles(
    mut commands: Commands,
    time: Res<Time<Fixed>>,
    mut projectiles: Query<(Entity, &mut RigidProjectile, &Position)>,
) {
    for (entity, mut projectile, position) in &mut projectiles {
        projectile.age += time.delta_secs();
        if projectile.age > 20.0 || position.y < -20.0 {
            commands.entity(entity).despawn();
        }
    }
}

fn impact_damage(world: &mut World) {
    let mut state = world.remove_resource::<ImpactState>().unwrap();
    let capture = world
        .get_resource::<ForceCapture>()
        .is_some_and(|capture| capture.enabled);
    let captured_at = world.resource::<Time<Fixed>>().elapsed_secs_f64();
    let mut impulse_samples = Vec::new();
    let mut touching = BTreeSet::new();
    let mut hits = BTreeMap::<(Entity, String), Vec3>::new();
    let mut shocks = BTreeMap::<(Entity, (Entity, Entity)), ClusterImpact>::new();
    let mut contact_count = 0;
    let mut max_impulse = 0.0_f32;
    for pair in world.resource::<ContactGraph>().iter_active_touching() {
        let key = if pair.collider1 < pair.collider2 {
            (pair.collider1, pair.collider2)
        } else {
            (pair.collider2, pair.collider1)
        };
        if state.touching.contains(&key) {
            touching.insert(key);
            continue;
        }
        let speed = pair
            .manifolds
            .iter()
            .flat_map(|manifold| &manifold.points)
            .map(|point| -point.normal_speed)
            .fold(0.0_f32, f32::max);
        if speed < IMPACT_MIN_SPEED {
            continue;
        }
        let impulse = pair.total_normal_impulse();
        let magnitude = impulse.length();
        if magnitude <= 0.0 {
            continue;
        }
        let mut structural_contact = false;
        for (collider, impulse, sign) in [
            (pair.collider1, -impulse, -1.0),
            (pair.collider2, impulse, 1.0),
        ] {
            if let Some(identity) = world.get::<ChunkIdentity>(collider) {
                let Some(instance) = world.get::<BuildingInstance>(identity.building) else {
                    continue;
                };
                let chunk = &instance.chunks[&identity.id];
                if !chunk.alive {
                    continue;
                }
                let Some(body) = chunk.cluster else {
                    continue;
                };
                let cluster = world.get::<Cluster>(body).unwrap();
                if capture {
                    for manifold in &pair.manifolds {
                        for point in &manifold.points {
                            if impulse_samples.len() < MAX_IMPULSE_SAMPLES
                                && point.normal_impulse > 0.0
                            {
                                impulse_samples.push(ImpulseSample {
                                    building: identity.building,
                                    chunk: identity.id.clone(),
                                    impulse: manifold.normal * (point.normal_impulse * sign),
                                    captured_at,
                                });
                            }
                        }
                    }
                }
                if world.get::<RigidBody>(body) == Some(&RigidBody::Dynamic)
                    && cluster.chunks.len() > 1
                {
                    let center = chunk.center_in_building().as_dvec3();
                    let origin = world.get::<Position>(body).unwrap().0;
                    let inverse_rotation = world.get::<Rotation>(body).unwrap().0.conjugate();
                    // 同一接触对的逐点冲量共享一次工况。其他接触有独立的受载来源。
                    let shock = shocks.entry((body, key)).or_insert_with(|| ClusterImpact {
                        building: identity.building,
                        chunks: cluster.chunks.clone(),
                        loads: BTreeMap::new(),
                        prepared: None,
                    });
                    let load = shock.loads.entry(identity.id.clone()).or_default();
                    for manifold in &pair.manifolds {
                        for point in &manifold.points {
                            let impulse = (inverse_rotation
                                * manifold.normal
                                * (point.normal_impulse * sign))
                                .as_dvec3();
                            let contact = (inverse_rotation * (point.point - origin)).as_dvec3();
                            load.linear += impulse;
                            load.angular += (contact - center).cross(impulse);
                        }
                    }
                } else {
                    *hits
                        .entry((identity.building, identity.id.clone()))
                        .or_default() += impulse;
                }
                structural_contact = true;
            }
        }
        if structural_contact {
            touching.insert(key);
            contact_count += 1;
            max_impulse = max_impulse.max(magnitude);
        }
    }
    state.touching = touching;
    state.shocks.extend(shocks.into_values());
    if let Some(mut samples) = world.get_resource_mut::<ForceSamples>() {
        for sample in impulse_samples {
            samples.record_impulse(sample);
        }
    }
    for ((building, id), impulse) in hits {
        let Some(instance) = world.get::<BuildingInstance>(building) else {
            continue;
        };
        let Some(asset) = world
            .resource::<Assets<DestructibleAsset>>()
            .get(&instance.asset)
        else {
            continue;
        };
        let chunk = &instance.chunks[&id];
        if !chunk.alive {
            continue;
        }
        let Some(body) = chunk.cluster else {
            continue;
        };
        let impulse = world.get::<Rotation>(body).unwrap().0.conjugate() * impulse;
        queue_local_damage(
            &asset.manifest,
            instance,
            building,
            &id,
            impulse,
            &mut state.pending,
        );
    }
    let mut structural = process_dynamic_impact(world, &mut state);
    let mut broken = BTreeMap::<Entity, Vec<String>>::new();
    let mut budget = IMPACT_BREAK_LIMIT;
    let mut structural_broken = 0;
    if let Some(report) = &mut structural
        && let Some((building, bonds)) = report.fracture.take()
        && let Some(mut instance) = world.get_mut::<BuildingInstance>(building)
    {
        for id in bonds {
            if let Some(active) = instance.bonds.get_mut(&id)
                && *active
            {
                *active = false;
                broken.entry(building).or_default().push(id);
                structural_broken += 1;
                budget -= 1;
            }
        }
        if structural_broken > 0 {
            instance.topology_dirty = true;
            if let Some(shock) = report.continuation.take() {
                state.shocks.push_back(shock);
            }
        }
    }
    let mut pending = state
        .pending
        .iter()
        .map(|(key, &utilization)| (key.clone(), utilization))
        .collect::<Vec<_>>();
    pending.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    for ((building, id), _) in pending {
        let Some(mut instance) = world.get_mut::<BuildingInstance>(building) else {
            state.pending.remove(&(building, id));
            continue;
        };
        if !instance.bonds.get(&id).copied().unwrap_or(false) {
            state.pending.remove(&(building, id));
            continue;
        }
        if budget == 0 {
            break;
        }
        instance.bonds.insert(id.clone(), false);
        instance.topology_dirty = true;
        broken.entry(building).or_default().push(id.clone());
        state.pending.remove(&(building, id));
        budget -= 1;
    }
    for (building, ids) in &broken {
        crate::interaction::bond_feedback(world, *building, ids);
    }
    if let Some(sample) = structural
        .as_mut()
        .and_then(|report| report.force_sample.take())
        && let Some(mut samples) = world.get_resource_mut::<ForceSamples>()
    {
        samples.record_force(sample);
    }
    let mut stats = world.resource_mut::<ImpactStats>();
    stats.contacts += contact_count;
    stats.last_impulse = max_impulse;
    stats.max_impulse = stats.max_impulse.max(max_impulse);
    if let Some(report) = structural {
        stats.structural_solves += 1;
        stats.structural_failures += u64::from(!report.valid);
        stats.max_structural_solve_ms = stats.max_structural_solve_ms.max(report.elapsed_ms);
        stats.max_force_balance_ratio = stats
            .max_force_balance_ratio
            .max(report.force_balance_ratio);
        stats.max_moment_balance_ratio = stats
            .max_moment_balance_ratio
            .max(report.moment_balance_ratio);
        stats.max_gauge_reaction_ratio = stats
            .max_gauge_reaction_ratio
            .max(report.gauge_reaction_ratio);
        stats.max_structural_utilization =
            stats.max_structural_utilization.max(report.max_utilization);
    }
    stats.pending_structural = state.shocks.len();
    stats.structural_broken_bonds += structural_broken;
    stats.released_structural_events = state.released;
    stats.rebound_fresh_structural_events = state.rebound_fresh;
    stats.invalidated_structural_events = state.invalidated;
    stats.broken_bonds += broken.values().map(|ids| ids.len() as u64).sum::<u64>();
    world.insert_resource(state);
}

fn queue_local_damage(
    manifest: &Manifest,
    instance: &BuildingInstance,
    building: Entity,
    id: &str,
    impulse: Vec3,
    pending: &mut BTreeMap<(Entity, String), f32>,
) {
    let bonds = manifest
        .bonds
        .iter()
        .filter(|bond| (bond.a == id || bond.b == id) && instance.bonds[&bond.id])
        .collect::<Vec<_>>();
    let total_area = bonds
        .iter()
        .flat_map(|bond| &bond.patches)
        .map(|patch| patch.area)
        .sum::<f32>();
    for bond in bonds {
        let material = |id: &str| &manifest.materials[&instance.chunks[id].material];
        let a = material(&bond.a);
        let b = material(&bond.b);
        let utilization = |normal: Vec3| {
            let projected = impulse.dot(normal);
            let tension = projected * if bond.a == id { -1.0 } else { 1.0 };
            let normal_strength = if tension > 0.0 {
                a.tension_strength.min(b.tension_strength)
            } else {
                a.compression_strength.min(b.compression_strength)
            };
            let normal_utilization =
                tension.abs() / (total_area * IMPACT_DURATION * normal_strength);
            let shear_utilization = (impulse - normal * projected).length()
                / (total_area * IMPACT_DURATION * a.shear_strength.min(b.shear_strength));
            normal_utilization.max(shear_utilization)
        };
        // Area-weighted facet impulses share the same traction; preserve each facet's direction.
        let utilization = bond
            .patches
            .iter()
            .map(|patch| utilization(Vec3::from(patch.normal)))
            .fold(0.0_f32, f32::max);
        if utilization >= 1.0 {
            let prior = pending.entry((building, bond.id.clone())).or_default();
            *prior = prior.max(utilization);
        }
    }
}

struct InertialLoadCase {
    loads: BTreeMap<String, NodeLoad>,
    force_scale: f64,
    moment_scale: f64,
    force_balance_ratio: f64,
    moment_balance_ratio: f64,
}

struct ImpactChunk<'a> {
    id: &'a str,
    mass: f64,
    center: DVec3,
    node: DVec3,
    inertia: DMat3,
}

fn inertial_loads(
    manifest: &Manifest,
    instance: &BuildingInstance,
    chunks: &[String],
    impulses: &BTreeMap<String, ImpulseLoad>,
) -> Option<InertialLoadCase> {
    let selected = chunks.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let members = manifest
        .chunks
        .iter()
        .filter(|record| selected.contains(record.id.as_str()))
        .map(|record| {
            let chunk = instance.chunks.get(&record.id)?;
            let properties = &chunk.mass_properties;
            let inertia = properties.global_angular_inertia_tensor(chunk.transform.rotation);
            Some(ImpactChunk {
                id: &record.id,
                mass: f64::from(properties.mass),
                center: chunk.center_in_building().as_dvec3(),
                node: stress::node_center(record),
                inertia: Mat3::from(inertia).as_dmat3(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    if members.len() != selected.len() {
        return None;
    }
    let mass = members.iter().map(|member| member.mass).sum::<f64>();
    let center = members
        .iter()
        .map(|member| member.center * member.mass)
        .sum::<DVec3>()
        / mass;
    let mut inertia = DMat3::ZERO;
    let mut linear = DVec3::ZERO;
    let mut angular = DVec3::ZERO;
    let mut force_scale = 0.0;
    let mut moment_scale = 0.0;
    for member in &members {
        let r = member.center - center;
        let outer = DMat3::from_cols(r * r.x, r * r.y, r * r.z);
        inertia += member.inertia + (DMat3::IDENTITY * r.length_squared() - outer) * member.mass;
        let impulse = impulses.get(member.id).copied().unwrap_or_default();
        linear += impulse.linear;
        angular += impulse.angular + r.cross(impulse.linear);
        force_scale += impulse.linear.length();
        moment_scale += impulse.angular.length() + r.cross(impulse.linear).length();
    }
    if !inertia.is_finite() || inertia.determinant() <= 0.0 {
        return None;
    }
    let delta_linear = linear / mass;
    let delta_angular = inertia.inverse() * angular;
    let duration = f64::from(IMPACT_DURATION);
    force_scale = (force_scale / duration).max(1.0);
    let length = (inertia.x_axis.x + inertia.y_axis.y + inertia.z_axis.z).sqrt() / mass.sqrt();
    moment_scale = (moment_scale / duration).max(force_scale * length).max(1.0);
    let mut loads = BTreeMap::new();
    let mut force_residual = DVec3::ZERO;
    let mut moment_residual = DVec3::ZERO;
    for member in members {
        let r = member.center - center;
        let impulse = impulses.get(member.id).copied().unwrap_or_default();
        let force =
            (impulse.linear - (delta_linear + delta_angular.cross(r)) * member.mass) / duration;
        let moment = (impulse.angular - member.inertia * delta_angular) / duration;
        let load = NodeLoad {
            force: force.to_array(),
            moment: moment.to_array(),
        }
        .at_reference(member.center - member.node);
        force_residual += force;
        moment_residual += (member.node - center).cross(force) + DVec3::from_array(load.moment);
        loads.insert(member.id.to_owned(), load);
    }
    Some(InertialLoadCase {
        loads,
        force_scale,
        moment_scale,
        force_balance_ratio: force_residual.length() / force_scale,
        moment_balance_ratio: moment_residual.length() / moment_scale,
    })
}

struct DynamicImpactReport {
    valid: bool,
    elapsed_ms: f64,
    force_balance_ratio: f64,
    moment_balance_ratio: f64,
    gauge_reaction_ratio: f64,
    max_utilization: f64,
    fracture: Option<(Entity, Vec<String>)>,
    continuation: Option<ClusterImpact>,
    force_sample: Option<DynamicForceSample>,
}

/// 从同一载荷解中选择可以同步起裂的连接，共用碎块的候选须等待下一次重算。
fn simultaneous_fractures(manifest: &Manifest, solution: &stress::StressSolution) -> Vec<String> {
    let mut candidates = manifest
        .bonds
        .iter()
        .filter_map(|bond| {
            let stress = solution.bonds.get(&bond.id)?;
            (stress.utilization >= 1.0).then_some((bond, stress.strain_energy))
        })
        .collect::<Vec<_>>();
    candidates.sort_unstable_by(|a, b| b.1.total_cmp(&a.1).then(a.0.id.cmp(&b.0.id)));

    let mut selected_chunks = BTreeSet::new();
    let mut fractures = Vec::new();
    for (bond, _) in candidates {
        if selected_chunks.contains(&bond.a) || selected_chunks.contains(&bond.b) {
            continue;
        }
        selected_chunks.insert(&bond.a);
        selected_chunks.insert(&bond.b);
        fractures.push(bond.id.clone());
        if fractures.len() == DYNAMIC_IMPACT_BREAK_LIMIT {
            break;
        }
    }
    fractures
}

fn process_dynamic_impact(world: &World, state: &mut ImpactState) -> Option<DynamicImpactReport> {
    while let Some(mut shock) = state.shocks.pop_front() {
        let Some(instance) = world.get::<BuildingInstance>(shock.building) else {
            continue;
        };
        let Some(asset) = world
            .resource::<Assets<DestructibleAsset>>()
            .get(&instance.asset)
        else {
            continue;
        };
        if shock.prepared.is_none() {
            let source_bodies = shock
                .loads
                .keys()
                .map(|id| {
                    instance
                        .chunks
                        .get(id)
                        .filter(|chunk| chunk.alive)
                        .and_then(|chunk| chunk.cluster)
                })
                .collect::<Option<Vec<_>>>();
            let Some(source_bodies) = source_bodies.filter(|bodies| !bodies.is_empty()) else {
                state.invalidated += 1;
                continue;
            };
            let body = source_bodies[0];
            if source_bodies.iter().any(|&source| source != body) {
                state.released += 1;
                continue;
            }
            let Some(cluster) = world.get::<Cluster>(body).filter(|cluster| {
                cluster.building == shock.building
                    && !cluster.anchored
                    && world.get::<RigidBody>(body) == Some(&RigidBody::Dynamic)
            }) else {
                state.released += 1;
                continue;
            };
            if shock.loads.keys().any(|id| !cluster.chunks.contains(id)) {
                state.invalidated += 1;
                continue;
            }
            if cluster.chunks != shock.chunks {
                let original = shock.chunks.iter().collect::<BTreeSet<_>>();
                if cluster.chunks.iter().any(|id| !original.contains(id)) {
                    state.invalidated += 1;
                    continue;
                }
                // An independent hit has not been solved yet. Bind its unchanged
                // contact loads to the one current child that contains every source.
                shock.chunks = cluster.chunks.clone();
                state.rebound_fresh += 1;
            }
        }
        let mut groups = BTreeMap::<Entity, Vec<String>>::new();
        for id in &shock.chunks {
            if let Some(chunk) = instance.chunks.get(id).filter(|chunk| chunk.alive)
                && let Some(body) = chunk.cluster
            {
                groups.entry(body).or_default().push(id.clone());
            }
        }
        if groups.values().map(Vec::len).sum::<usize>() != shock.chunks.len() {
            state.invalidated += 1;
            continue;
        }
        if groups.len() != 1 {
            // A solved parent response ends at separation. Only an independent,
            // never-solved hit may bind once to its current source-containing child.
            state.released += 1;
            continue;
        }
        if shock.chunks.len() < 2 {
            continue;
        }
        let started = bevy::platform::time::Instant::now();
        // Continuations retain these inputs until the membership checks above release the event.
        if shock.prepared.is_none() {
            let Some(case) = inertial_loads(&asset.manifest, instance, &shock.chunks, &shock.loads)
            else {
                continue;
            };
            shock.prepared = Some(case);
        }
        let case = shock.prepared.as_ref().unwrap();
        let mut report = DynamicImpactReport {
            valid: false,
            elapsed_ms: 0.0,
            force_balance_ratio: case.force_balance_ratio,
            moment_balance_ratio: case.moment_balance_ratio,
            gauge_reaction_ratio: f64::INFINITY,
            max_utilization: 0.0,
            fracture: None,
            continuation: None,
            force_sample: None,
        };
        let result = if case.force_balance_ratio <= 1e-6 && case.moment_balance_ratio <= 1e-6 {
            stress::solve_with_gauge(
                &asset.manifest,
                &shock.chunks,
                &instance.bonds,
                &case.loads,
                &SolverSettings::default(),
                &shock.chunks[0],
            )
        } else {
            Err(stress::SolveError::InvalidModel(
                "Unbalanced inertial impact loads".into(),
            ))
        };
        match result {
            Ok(solution) => {
                let reaction = &solution.reactions[&shock.chunks[0]];
                report.gauge_reaction_ratio = (DVec3::from_array(reaction.force).length()
                    / case.force_scale)
                    .max(DVec3::from_array(reaction.moment).length() / case.moment_scale);
                if report.gauge_reaction_ratio <= 1e-5 {
                    report.valid = true;
                    report.max_utilization = solution
                        .bonds
                        .values()
                        .map(|bond| bond.utilization)
                        .fold(0.0, f64::max);
                    let fractures = simultaneous_fractures(&asset.manifest, &solution);
                    if !fractures.is_empty() {
                        report.fracture = Some((shock.building, fractures));
                    }
                    if world
                        .get_resource::<ForceCapture>()
                        .is_some_and(|capture| capture.enabled)
                    {
                        report.force_sample = Some(DynamicForceSample {
                            building: shock.building,
                            body: *groups.keys().next().unwrap(),
                            chunks: shock.chunks.clone(),
                            bonds: solution.bonds,
                            bond_signature: bond_signature(&instance.bonds),
                            captured_at: world.resource::<Time<Fixed>>().elapsed_secs_f64(),
                        });
                    }
                }
            }
            Err(error) => warn!(
                "Dynamic impact stress failed for {:?}: {error}",
                shock.building
            ),
        }
        if !report.valid {
            for (id, load) in &shock.loads {
                queue_local_damage(
                    &asset.manifest,
                    instance,
                    shock.building,
                    id,
                    load.linear.as_vec3(),
                    &mut state.pending,
                );
            }
        }
        if report.fracture.is_some() {
            // 下一批断裂须等待拓扑重建并重新求解。
            report.continuation = Some(shock);
        }
        report.elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        return Some(report);
    }
    None
}

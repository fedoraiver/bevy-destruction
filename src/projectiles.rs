use std::collections::{BTreeMap, BTreeSet, VecDeque};

use avian3d::{collision::contact_types::ContactGraph, prelude::*};
use bevy::prelude::*;

use crate::{
    DemoOptions,
    asset::DestructibleAsset,
    camera::CameraControl,
    scene::{BuildingInstance, ChunkIdentity, DemoCamera},
};

#[cfg(test)]
#[path = "projectile_tests.rs"]
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
pub struct LaunchRequests(pub VecDeque<LaunchRequest>);

#[derive(Component)]
pub struct RigidProjectile {
    age: f32,
}

#[derive(Resource, Default)]
pub struct ImpactStats {
    pub launched: u64,
    pub contacts: u64,
    pub broken_bonds: u64,
    pub last_impulse: f32,
    pub max_impulse: f32,
}

#[derive(Resource, Default)]
struct ImpactState {
    touching: BTreeSet<(Entity, Entity)>,
    pending: BTreeMap<(Entity, String), f32>,
}

pub(crate) fn reset(world: &mut World) {
    world.resource_mut::<LaunchRequests>().0.clear();
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
        requests.0.push_back(LaunchRequest {
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
    for request in requests.0.drain(..) {
        if count >= 12 {
            continue;
        }
        let (mesh, collider, inertia) = match request.shape {
            ProjectileShape::Sphere => (
                Mesh::from(Sphere::new(0.65)),
                Collider::sphere(0.65),
                PROJECTILE_MASS * 0.4 * 0.65 * 0.65,
            ),
            ProjectileShape::Cube => (
                Mesh::from(Cuboid::from_length(1.2)),
                Collider::cuboid(1.2, 1.2, 1.2),
                PROJECTILE_MASS * 1.2 * 1.2 / 6.0,
            ),
        };
        commands
            .spawn((
                Name::new("Unbreakable rigid impactor"),
                RigidProjectile { age: 0.0 },
                RigidBody::Dynamic,
                collider,
                ColliderDensity(0.0),
                Mass(PROJECTILE_MASS),
                CenterOfMass(Vec3::ZERO),
                AngularInertia::from_mat3_unchecked(Mat3::from_diagonal(Vec3::splat(inertia))),
                NoAutoMass,
                NoAutoAngularInertia,
                NoAutoCenterOfMass,
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
    let mut touching = BTreeSet::new();
    let mut hits = BTreeMap::<(Entity, String), Vec3>::new();
    let mut contact_count = 0;
    let mut max_impulse = 0.0_f32;
    for pair in world.resource::<ContactGraph>().iter_active_touching() {
        let key = if pair.collider1 < pair.collider2 {
            (pair.collider1, pair.collider2)
        } else {
            (pair.collider2, pair.collider1)
        };
        touching.insert(key);
        if state.touching.contains(&key) {
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
        for (collider, impulse) in [(pair.collider1, -impulse), (pair.collider2, impulse)] {
            if let Some(identity) = world.get::<ChunkIdentity>(collider) {
                *hits
                    .entry((identity.building, identity.id.clone()))
                    .or_default() += impulse;
                structural_contact = true;
            }
        }
        if structural_contact {
            contact_count += 1;
            max_impulse = max_impulse.max(magnitude);
        }
    }
    state.touching = touching;
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
        let bonds = asset
            .manifest
            .bonds
            .iter()
            .filter(|bond| (bond.a == id || bond.b == id) && instance.bonds[&bond.id])
            .collect::<Vec<_>>();
        let total_area = bonds.iter().map(|bond| bond.area).sum::<f32>();
        for bond in bonds {
            let material = |id: &str| &asset.manifest.materials[&instance.chunks[id].material];
            let a = material(&bond.a);
            let b = material(&bond.b);
            let normal = Vec3::from(bond.normal);
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
            let utilization = normal_utilization.max(shear_utilization);
            if utilization >= 1.0 {
                let pending = state
                    .pending
                    .entry((building, bond.id.clone()))
                    .or_default();
                *pending = pending.max(utilization);
            }
        }
    }
    let mut pending = state
        .pending
        .iter()
        .map(|(key, &utilization)| (key.clone(), utilization))
        .collect::<Vec<_>>();
    pending.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut broken = BTreeMap::<Entity, Vec<String>>::new();
    let mut budget = IMPACT_BREAK_LIMIT;
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
    let mut stats = world.resource_mut::<ImpactStats>();
    stats.contacts += contact_count;
    stats.last_impulse = max_impulse;
    stats.max_impulse = stats.max_impulse.max(max_impulse);
    stats.broken_bonds += broken.values().map(|ids| ids.len() as u64).sum::<u64>();
    world.insert_resource(state);
}

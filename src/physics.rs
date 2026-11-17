use std::collections::{BTreeMap, BTreeSet};

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{asset::DestructibleAsset, scene::BuildingInstance, structure};

#[cfg(test)]
#[path = "physics_tests.rs"]
mod tests;

pub struct DestructionPhysicsPlugin;

impl Plugin for DestructionPhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, ground).add_systems(
            FixedPostUpdate,
            rebuild_clusters.before(PhysicsSystems::First),
        );
    }
}

#[derive(Component, Clone)]
pub struct Cluster {
    pub building: Entity,
    pub chunks: Vec<String>,
    pub anchored: bool,
}

fn ground(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Name::new("Permanent ground"),
        RigidBody::Static,
        Collider::cuboid(400.0, 0.2, 400.0),
        Mesh3d(meshes.add(Cuboid::new(400.0, 0.2, 400.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.12, 0.14, 0.16),
            perceptual_roughness: 1.0,
            ..default()
        })),
        Transform::from_xyz(0.0, -0.1, 0.0),
        Friction::new(0.7),
        Restitution::new(0.0),
    ));
}

#[derive(Clone, Copy)]
struct Motion {
    position: Vec3,
    rotation: Quat,
    center: Vec3,
    linear: Vec3,
    angular: Vec3,
}

impl Motion {
    fn transform(self) -> Transform {
        Transform::from_translation(self.position).with_rotation(self.rotation)
    }
}

pub(crate) fn global_transform(world: &World, entity: Entity) -> GlobalTransform {
    let mut ancestors = vec![entity];
    let mut parent = world.get::<ChildOf>(entity);
    while let Some(child_of) = parent {
        ancestors.push(child_of.parent());
        parent = world.get::<ChildOf>(child_of.parent());
    }
    ancestors
        .into_iter()
        .rev()
        .fold(GlobalTransform::IDENTITY, |global, entity| {
            global * *world.get::<Transform>(entity).unwrap()
        })
}

pub(crate) fn rebuild_clusters(world: &mut World) {
    let buildings = world
        .query::<(Entity, &BuildingInstance)>()
        .iter(world)
        .filter(|(_, instance)| instance.topology_dirty && instance.bounds.is_some())
        .map(|(entity, _)| entity)
        .collect::<Vec<_>>();
    for building in buildings {
        let instance = world.get::<BuildingInstance>(building).unwrap();
        let Some(asset) = world
            .resource::<Assets<DestructibleAsset>>()
            .get(&instance.asset)
        else {
            continue;
        };
        let manifest = &asset.manifest;
        let alive = instance
            .chunks
            .iter()
            .filter(|(_, chunk)| chunk.alive)
            .map(|(id, _)| id.clone())
            .collect::<BTreeSet<_>>();
        let components = structure::components(manifest, &alive, &instance.bonds);
        let chunks = instance
            .chunks
            .iter()
            .map(|(id, chunk)| (id.clone(), (chunk.entity, chunk.cluster, chunk.transform)))
            .collect::<BTreeMap<_, _>>();
        let previous = instance.clusters.clone();
        let root_global = global_transform(world, building);
        let root_pose = root_global.compute_transform();
        let initial = Motion {
            position: root_pose.translation,
            rotation: root_pose.rotation,
            center: root_pose.translation,
            linear: Vec3::ZERO,
            angular: Vec3::ZERO,
        };
        let motions = previous
            .iter()
            .map(|&entity| {
                let position = world.get::<Position>(entity).unwrap().0;
                let rotation = world.get::<Rotation>(entity).unwrap().0;
                let center = position + rotation * world.get::<CenterOfMass>(entity).unwrap().0;
                (
                    entity,
                    Motion {
                        position,
                        rotation,
                        center,
                        linear: world.get::<LinearVelocity>(entity).unwrap().0,
                        angular: world.get::<AngularVelocity>(entity).unwrap().0,
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();

        let retained = components
            .iter()
            .filter_map(|component| {
                let parent = chunks[&component.chunks[0]].1?;
                let cluster = world.get::<Cluster>(parent)?;
                (cluster.chunks == component.chunks && cluster.anchored == component.anchored)
                    .then_some(parent)
            })
            .collect::<BTreeSet<_>>();
        let masses = components
            .iter()
            .map(|component| {
                let parent = chunks[&component.chunks[0]].1;
                if parent.is_some_and(|parent| retained.contains(&parent)) {
                    None
                } else {
                    Some(structure::cluster_mass(manifest, &component.chunks))
                }
            })
            .collect::<Vec<_>>();
        // Purge contacts with old body owners before rebinding colliders.
        for &(entity, parent, _) in chunks.values() {
            if !parent.is_some_and(|parent| retained.contains(&parent)) {
                world.entity_mut(entity).insert(ColliderDisabled);
            }
        }
        world.flush();

        for (id, &(entity, _, _)) in &chunks {
            if !alive.contains(id) {
                world.entity_mut(entity).remove::<Collider>();
                world.entity_mut(entity).insert(ChildOf(building));
                world
                    .get_mut::<BuildingInstance>(building)
                    .unwrap()
                    .chunks
                    .get_mut(id)
                    .unwrap()
                    .cluster = None;
            }
        }
        world.flush();

        let mut next = Vec::new();
        for (component, mass) in components.into_iter().zip(masses) {
            let parent = chunks[&component.chunks[0]].1;
            if let Some(parent) = parent.filter(|parent| retained.contains(parent)) {
                next.push(parent);
                continue;
            }
            let mass = mass.unwrap();
            let motion = parent.map_or(initial, |parent| motions[&parent]);
            let pose = motion.transform();
            let global = GlobalTransform::from(pose);
            let center = pose.transform_point(mass.center);
            let body = world
                .spawn((
                    Name::new(if component.anchored {
                        "Anchored building cluster"
                    } else {
                        "Detached building cluster"
                    }),
                    global.reparented_to(&root_global),
                    global,
                    Visibility::Inherited,
                    ChildOf(building),
                    if component.anchored {
                        RigidBody::Static
                    } else {
                        RigidBody::Dynamic
                    },
                    Position(motion.position),
                    Rotation(motion.rotation),
                    Mass(mass.mass),
                    CenterOfMass(mass.center),
                    AngularInertia::from_mat3_unchecked(mass.inertia),
                    NoAutoMass,
                    NoAutoAngularInertia,
                    NoAutoCenterOfMass,
                ))
                .insert((
                    LinearVelocity(structure::inherited_velocity(
                        motion.linear,
                        motion.angular,
                        motion.center,
                        center,
                    )),
                    AngularVelocity(motion.angular),
                    Friction::new(0.7),
                    Restitution::new(0.0),
                    Cluster {
                        building,
                        chunks: component.chunks.clone(),
                        anchored: component.anchored,
                    },
                ))
                .id();
            if !component.anchored {
                world.entity_mut(body).insert(TransformInterpolation);
            }
            for id in component.chunks {
                let (entity, _, local) = chunks[&id];
                world
                    .entity_mut(entity)
                    .insert((local, global * local, ChildOf(body)));
                world.entity_mut(entity).insert(ColliderOf { body });
                world
                    .get_mut::<BuildingInstance>(building)
                    .unwrap()
                    .chunks
                    .get_mut(&id)
                    .unwrap()
                    .cluster = Some(body);
            }
            next.push(body);
        }
        world.flush();
        for old in previous {
            if !next.contains(&old) {
                world.despawn(old);
            }
        }
        for (id, &(entity, _, _)) in &chunks {
            if alive.contains(id) {
                world.entity_mut(entity).remove::<ColliderDisabled>();
            }
        }
        let mut instance = world.get_mut::<BuildingInstance>(building).unwrap();
        instance.clusters = next;
        instance.detail_dirty.extend(alive);
        instance.topology_dirty = false;
    }
}

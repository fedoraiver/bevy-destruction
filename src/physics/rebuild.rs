use std::collections::{BTreeMap, BTreeSet};

use avian3d::prelude::*;
use bevy::{ecs::system::SystemState, prelude::*};

use crate::{
    asset::DestructibleAsset,
    scene::{BondInstance, BuildingInstance, ChunkInstance, RefreshDamageDetails},
    structure,
};

use super::{Cluster, NeedsClusterRebuild, building_clusters};

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

pub(crate) fn rebuild_clusters(
    world: &mut World,
    buildings: &mut QueryState<(Entity, &'static BuildingInstance), With<NeedsClusterRebuild>>,
    transforms: &mut SystemState<TransformHelper<'static, 'static>>,
    mass_properties: &mut SystemState<MassPropertyHelper<'static, 'static>>,
) {
    let buildings = buildings
        .iter(world)
        .filter(|(_, instance)| instance.bounds.is_some())
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
        let mut previous = building_clusters(world, building).collect::<Vec<_>>();
        // Match the component order when retiring bodies so later entity allocation stays stable.
        previous.sort_unstable_by(|&left, &right| {
            let left = world.get::<Cluster>(left).unwrap();
            let right = world.get::<Cluster>(right).unwrap();
            left.chunks.first().cmp(&right.chunks.first())
        });
        let root_global = transforms
            .get(world)
            .expect("TransformHelper's read-only queries are available")
            .compute_global_transform(building)
            .expect("A loaded building has a valid transform hierarchy");
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
                let center =
                    position + rotation * world.get::<ComputedCenterOfMass>(entity).unwrap().0;
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

        let mut alive = BTreeSet::new();
        let mut chunks = BTreeMap::new();
        for (id, &entity) in &instance.chunks {
            let chunk = world.get::<ChunkInstance>(entity).unwrap();
            if chunk.alive {
                alive.insert(id.clone());
            }
            let parent = world
                .get::<ColliderOf>(entity)
                .map(|owner| owner.body)
                .filter(|body| motions.contains_key(body));
            chunks.insert(id.clone(), (entity, parent, chunk.transform));
        }
        let bonds =
            instance.bond_states(|entity| world.get::<BondInstance>(entity).unwrap().active);
        let components = structure::components(manifest, &alive, &bonds);

        let retained = components
            .iter()
            .filter_map(|component| {
                let parent = chunks[&component.chunks[0]].1?;
                let cluster = world.get::<Cluster>(parent)?;
                (cluster.chunks == component.chunks && cluster.anchored == component.anchored)
                    .then_some(parent)
            })
            .collect::<BTreeSet<_>>();
        // Purge contacts with old body owners before rebinding colliders.
        for &(entity, parent, _) in chunks.values() {
            if !parent.is_some_and(|parent| retained.contains(&parent)) {
                world.entity_mut(entity).insert(ColliderDisabled);
            }
        }
        world.flush();

        for (id, &(entity, _, _)) in &chunks {
            if !alive.contains(id) {
                world.entity_mut(entity).remove::<(Collider, ColliderOf)>();
                world.entity_mut(entity).insert(ChildOf(building));
            }
        }
        world.flush();

        let mut created = Vec::new();
        for component in components {
            let parent = chunks[&component.chunks[0]].1;
            if parent.is_some_and(|parent| retained.contains(&parent)) {
                continue;
            }
            let motion = parent.map_or(initial, |parent| motions[&parent]);
            let pose = motion.transform();
            let global = GlobalTransform::from(pose);
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
                ))
                .insert((
                    AngularVelocity(motion.angular),
                    Friction::new(0.7),
                    Restitution::new(0.0),
                    Cluster {
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
            }
            created.push((body, motion));
        }
        // Apply Avian's collider reparenting before any former owner despawns.
        world.flush();
        for old in previous {
            if !retained.contains(&old) {
                world.despawn(old);
            }
        }
        for (id, &(entity, _, _)) in &chunks {
            if alive.contains(id) {
                world.entity_mut(entity).remove::<ColliderDisabled>();
            }
        }
        if !created.is_empty() {
            world.flush();
            {
                let mut mass_properties = mass_properties
                    .get_mut(world)
                    .expect("MassPropertyHelper's queries are available");
                for &(body, _) in &created {
                    mass_properties.update_mass_properties(body);
                }
            }
            for (body, motion) in created {
                let local_center = world.get::<ComputedCenterOfMass>(body).unwrap().0;
                let center = motion.transform().transform_point(local_center);
                let linear = structure::inherited_velocity(
                    motion.linear,
                    motion.angular,
                    motion.center,
                    center,
                );
                world.entity_mut(body).insert(LinearVelocity(linear));
            }
        }
        let mut building = world.entity_mut(building);
        // Keep the rebuild step's change visible to consumers of the pre-rebuild solution.
        building
            .get_mut::<BuildingInstance>()
            .unwrap()
            .set_changed();
        building
            .remove::<NeedsClusterRebuild>()
            .insert(RefreshDamageDetails);
    }
}

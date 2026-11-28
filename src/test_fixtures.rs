use avian3d::prelude::*;
use bevy::prelude::*;

use crate::{
    asset::{
        AnchorRecord, BondPatchRecord, BondRecord, ChunkRecord, ColliderRecord, Coordinates,
        DestructibleAsset, Manifest, PhysicalMaterial,
    },
    fracture_detail::ContactPatch,
    physics::NeedsClusterRebuild,
    scene::{
        BondInstance, BuildingBounds, BuildingInstance, ChunkIdentity, ChunkInstance,
        ChunkPrimitive, RemovedChunk, spawn_bonds, spawn_patches,
    },
};

pub(crate) const LEFT_SUPPORT: &str = "a-left-column";
pub(crate) const RIGHT_SUPPORT: &str = "b-right-column";
pub(crate) const BEAM: &str = "c-beam";
pub(crate) const LEFT_BOND: &str = "left-beam";
pub(crate) const RIGHT_BOND: &str = "right-beam";

pub(crate) fn beam_manifest() -> Manifest {
    let mut manifest = Manifest {
        schema: "bevy_destruction_asset".into(),
        schema_version: 2,
        name: "Concrete beam with two narrow supports".into(),
        asset_id: "0".repeat(64),
        coordinates: Coordinates {
            units: "meters".into(),
            up: "+Y".into(),
            handedness: "right".into(),
        },
        materials: std::collections::BTreeMap::from([(
            "concrete".into(),
            PhysicalMaterial {
                density: 2400.0,
                color: [0.55, 0.57, 0.6, 1.0],
                tension_strength: 3e6,
                compression_strength: 30e6,
                shear_strength: 4e6,
                young_modulus: 30e9,
                poisson_ratio: 0.2,
            },
        )]),
        chunks: Vec::new(),
        bonds: Vec::new(),
        anchors: Vec::new(),
        mesh_file: "synthetic-beam.glb".into(),
        mesh_sha256: "0".repeat(64),
    };
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
        let volume = size.element_product();
        let mut chunk = ChunkRecord {
            id: id.into(),
            node: id.into(),
            part_id: id.into(),
            kind: if id == BEAM { "beam" } else { "column" }.into(),
            material: "concrete".into(),
            position: position.to_array(),
            rotation: Quat::IDENTITY.to_array(),
            center_of_mass: [0.0; 3],
            volume,
            mass: volume * manifest.materials["concrete"].density,
            inertia: [[0.0; 3]; 3],
            colliders: Vec::new(),
        };
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
        patches: vec![BondPatchRecord {
            area: 0.01,
            center: [x, 1.0, 0.0],
            normal: Vec3::Y.to_array(),
            area_covariance: Mat3::from_diagonal(Vec3::new(second_moment, 0.0, second_moment))
                .to_cols_array_2d(),
            vertices: vec![
                [x - 0.05, 1.0, -0.05],
                [x - 0.05, 1.0, 0.05],
                [x + 0.05, 1.0, 0.05],
                [x + 0.05, 1.0, -0.05],
            ],
        }],
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
    let patches = std::sync::Arc::new(
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
        let collider = Collider::cuboid(size.x, size.y, size.z);
        let density = manifest.materials[&record.material].density;
        let mass_properties = collider.mass_properties(density);
        let chunk = world
            .spawn((
                Name::new(record.id.clone()),
                local,
                GlobalTransform::from(pose) * local,
                Visibility::Inherited,
                ChildOf(root),
                collider,
                ColliderDensity(density),
                ChunkIdentity {
                    building: root,
                    id: record.id.clone(),
                },
            ))
            .id();
        let mesh = world
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::from_size(size));
        world.spawn((
            Name::new(format!("{} stress fixture primitive", record.id)),
            Mesh3d(mesh),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            ChildOf(chunk),
            ChunkPrimitive,
        ));
        world.entity_mut(chunk).insert(ChunkInstance {
            alive: true,
            triangles: Vec::new(),
            transform: local,
            mass_properties,
            material: "concrete".into(),
        });
        instance.chunks.insert(record.id, chunk);
    }
    let mut commands = world.commands();
    spawn_bonds(&mut commands, root, &mut instance, &manifest.bonds);
    spawn_patches(&mut commands, root, &mut instance, patches);
    world.flush();
    world
        .entity_mut(root)
        .insert((instance, NeedsClusterRebuild));
    root
}

pub(crate) fn remove_left_support(world: &mut World, root: Entity) {
    let (entity, bond) = {
        let instance = world.get::<BuildingInstance>(root).unwrap();
        (instance.chunks[LEFT_SUPPORT], instance.bonds[LEFT_BOND])
    };
    world.get_mut::<ChunkInstance>(entity).unwrap().alive = false;
    world.get_mut::<BondInstance>(bond).unwrap().active = false;
    world.entity_mut(root).insert(NeedsClusterRebuild);
    world
        .entity_mut(entity)
        .insert((RemovedChunk, Visibility::Hidden));
}

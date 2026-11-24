use super::*;
fn face(z: f32) -> [[Vec3; 3]; 1] {
    [[
        Vec3::new(0.0, 0.0, z),
        Vec3::new(10.0, 0.0, z),
        Vec3::new(0.0, 10.0, z),
    ]]
}
#[test]
fn selection_uses_surface_distance_and_rejects_back_wall() {
    let point = Vec3::new(0.05, 0.05, 0.0);
    assert!(surface_distance(&face(0.0), point, Vec3::Z).unwrap() < 0.001);
    assert!(surface_distance(&face(-0.2), point, Vec3::Z).is_none());
    assert!(surface_distance(&face(0.0), point, -Vec3::Z).is_none());
    assert!(
        surface_distance(&face(0.0), Vec3::new(-1.0, 0.0, 0.0), Vec3::Z).unwrap() > BREAK_RADIUS
    );
}

#[test]
fn triangle_projection_respects_finite_edges_and_the_face_tolerance() {
    let triangle = [Vec3::ZERO, Vec3::X, Vec3::Y];
    for (point, closest) in [
        (Vec3::new(0.25, -0.2, 0.03), Vec3::new(0.25, 0.0, 0.0)),
        (Vec3::new(1.4, -0.2, 0.03), Vec3::X),
        (Vec3::new(0.25, -5e-8, 0.03), Vec3::new(0.25, -5e-8, 0.0)),
    ] {
        assert_eq!(closest_on_triangle(triangle, point, Vec3::Z), closest);
    }
}

#[test]
fn tilted_surface_depth_is_measured_along_the_hit_normal() {
    let triangles = [[Vec3::ZERO, Vec3::X, Vec3::new(0.0, 0.8, 0.6)]];
    let center = Vec3::new(0.25, 0.2, 0.15);
    let geometric_normal = Vec3::new(0.0, -0.6, 0.8);
    let within = center + geometric_normal * 0.099;
    let beyond = center + geometric_normal * 0.101;
    let distance = surface_distance(&triangles, within, Vec3::Z).unwrap();
    assert!((distance - 0.099).abs() < 1e-6);
    assert!(surface_distance(&triangles, beyond, Vec3::Z).is_none());
}

#[test]
fn fracture_feedback_uses_each_moving_cluster_pose() {
    use crate::fracture_detail::ContactPatch;
    use crate::scene::{spawn_bonds, spawn_patches};
    let mut world = World::new();
    let mut manifest = crate::stress_smoke::beam_manifest();
    let template = manifest.bonds[0].clone();
    manifest.bonds = [("ab", "a", "b"), ("cd", "c", "d")]
        .map(|(id, a, b)| crate::asset::BondRecord {
            id: id.into(),
            a: a.into(),
            b: b.into(),
            center: [0.0; 3],
            normal: Vec3::Y.to_array(),
            patches: vec![crate::asset::BondPatchRecord {
                center: [0.0; 3],
                vertices: vec![
                    [-0.05, 0.0, -0.05],
                    [-0.05, 0.0, 0.05],
                    [0.05, 0.0, 0.05],
                    [0.05, 0.0, -0.05],
                ],
                ..template.patches[0].clone()
            }],
            ..template.clone()
        })
        .into();
    let root = world.spawn(Transform::IDENTITY).id();
    let mut instance = BuildingInstance::pending(Handle::default());
    let poses = [
        Transform::from_xyz(4.0, 2.0, 1.0).with_rotation(Quat::from_rotation_z(0.3)),
        Transform::from_xyz(-8.0, 3.0, -2.0).with_rotation(Quat::from_rotation_y(0.7)),
    ];
    let triangle = [Vec3::ZERO, Vec3::X, Vec3::Z];
    let mut patches = Vec::new();
    for ((id, a, b), pose) in [("ab", "a", "b"), ("cd", "c", "d")].into_iter().zip(poses) {
        let body = world
            .spawn((
                ChildOf(root),
                pose,
                GlobalTransform::from(pose),
                RigidBody::Dynamic,
                crate::physics::Cluster {
                    chunks: vec![a.into(), b.into()],
                    anchored: false,
                },
                Position(pose.translation),
                Rotation(pose.rotation),
            ))
            .id();
        for chunk_id in [a, b] {
            let entity = world
                .spawn((
                    ChildOf(body),
                    Transform::IDENTITY,
                    GlobalTransform::from(pose),
                    ColliderOf { body },
                    ChunkInstance {
                        alive: true,
                        triangles: Vec::new(),
                        transform: Transform::IDENTITY,
                        mass_properties: MassProperties3d::ZERO,
                        material: "concrete".into(),
                    },
                ))
                .id();
            instance.chunks.insert(chunk_id.into(), entity);
        }
        patches.push(ContactPatch {
            id: id.into(),
            bond_id: id.into(),
            a: a.into(),
            b: b.into(),
            normal: Vec3::Y,
            triangles: vec![triangle],
            area: 0.5,
            rims: Vec::new(),
        });
    }
    let mut commands = world.commands();
    spawn_bonds(&mut commands, root, &mut instance, &manifest.bonds);
    spawn_patches(
        &mut commands,
        root,
        &mut instance,
        std::sync::Arc::new(patches),
    );
    world.flush();
    let burst = build_bond_feedback(
        &instance,
        &manifest,
        &["ab".into(), "cd".into()],
        |entity| {
            (
                world.get::<ChunkInstance>(entity).unwrap(),
                world.get::<ColliderOf>(entity).map(|owner| owner.body),
            )
        },
        instance
            .patches
            .iter()
            .map(|&entity| world.get::<PatchInstance>(entity).unwrap().patch()),
        |body| {
            let body = body.unwrap();
            Transform::from_translation(world.get::<Position>(body).unwrap().0)
                .with_rotation(world.get::<Rotation>(body).unwrap().0)
        },
    )
    .unwrap();
    assert_eq!(burst.surfaces.len(), 2);
    for (surface, pose) in burst.surfaces.iter().zip(poses) {
        for (actual, local) in surface.triangle.iter().zip(triangle) {
            assert!(actual.distance(pose.transform_point(local)) < 1e-5);
        }
        assert!(surface.normal.distance(pose.rotation * Vec3::Y) < 1e-5);
    }
}

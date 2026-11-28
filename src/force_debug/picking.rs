use avian3d::prelude::{Collider, ColliderOf};
use bevy::{prelude::*, window::PrimaryWindow};

use crate::{
    asset::DestructibleAsset,
    camera::CameraControl,
    force_samples::ForceCapture,
    force_visibility::{ForceVisibilityCache, SurfaceHit, VisibilityChunk, VisibilityOccluder},
    physics::Cluster,
    scene::{BuildingInstance, ChunkIdentity, ChunkInstance, DemoCamera},
};

#[derive(Resource, Default)]
pub(super) struct Aim {
    pub(super) hit: Option<SurfaceHit>,
    pub(super) arrow_length: f32,
    pub(super) queried: bool,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn pick_target(
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

mod edges;

use bevy::{gizmos::config::GizmoConfigGroup, prelude::*};

pub use edges::fracture_edges;

#[cfg(test)]
use bevy::mesh::PrimitiveTopology;

#[derive(Component, Debug, Clone, Default)]
pub struct FractureEdges {
    pub segments: Vec<[Vec3; 2]>,
}

#[derive(Resource, Default)]
pub struct DebugOverlay {
    pub enabled: bool,
}

#[derive(Default, Reflect, GizmoConfigGroup)]
struct FractureGizmos;

pub struct FractureDebugPlugin;

impl Plugin for FractureDebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugOverlay>()
            .init_gizmo_group::<FractureGizmos>()
            .add_systems(Startup, configure_lines)
            .add_systems(Update, toggle_overlay)
            .add_systems(
                PostUpdate,
                draw_edges
                    .after(TransformSystems::Propagate)
                    .run_if(|overlay: Res<DebugOverlay>| overlay.enabled),
            );
    }
}

fn configure_lines(mut configs: ResMut<GizmoConfigStore>) {
    let (config, _) = configs.config_mut::<FractureGizmos>();
    config.depth_bias = -0.00002;
    config.line.width = 1.4;
}

fn toggle_overlay(keys: Res<ButtonInput<KeyCode>>, mut overlay: ResMut<DebugOverlay>) {
    if keys.just_pressed(KeyCode::Digit0) || keys.just_pressed(KeyCode::Numpad0) {
        overlay.enabled = !overlay.enabled;
    }
}

fn draw_edges(
    chunks: Query<(&FractureEdges, &GlobalTransform, &InheritedVisibility)>,
    mut gizmos: Gizmos<FractureGizmos>,
) {
    for (edges, transform, visibility) in &chunks {
        if !visibility.get() {
            continue;
        }
        for [a, b] in &edges.segments {
            gizmos.line(
                transform.transform_point(*a),
                transform.transform_point(*b),
                Color::srgb(1.0, 0.32, 0.055),
            );
        }
    }
}

#[cfg(test)]
mod tests;

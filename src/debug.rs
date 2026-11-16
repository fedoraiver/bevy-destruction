use std::collections::HashMap;

use bevy::{
    gizmos::config::GizmoConfigGroup,
    mesh::{Indices, PrimitiveTopology, VertexAttributeValues},
    prelude::*,
};

const WELD_TOLERANCE: f32 = 0.00001;
const COPLANAR_COSINE: f32 = 0.99996;

#[derive(Component, Debug, Clone, Default)]
pub struct FractureEdges(pub Vec<[Vec3; 2]>);

#[derive(Resource, Default)]
pub struct DebugOverlay(pub bool);

#[derive(Default, Reflect, GizmoConfigGroup)]
struct FractureGizmos;

pub struct FractureDebugPlugin;

impl Plugin for FractureDebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DebugOverlay>()
            .init_gizmo_group::<FractureGizmos>()
            .add_systems(Startup, configure_lines)
            .add_systems(Update, toggle_overlay)
            .add_systems(PostUpdate, draw_edges.after(TransformSystems::Propagate));
    }
}

fn configure_lines(mut configs: ResMut<GizmoConfigStore>) {
    let (config, _) = configs.config_mut::<FractureGizmos>();
    config.depth_bias = -0.00002;
    config.line.width = 1.4;
}

fn toggle_overlay(keys: Res<ButtonInput<KeyCode>>, mut overlay: ResMut<DebugOverlay>) {
    if keys.just_pressed(KeyCode::Digit0) || keys.just_pressed(KeyCode::Numpad0) {
        overlay.0 = !overlay.0;
    }
}

fn draw_edges(
    overlay: Res<DebugOverlay>,
    chunks: Query<(&FractureEdges, &GlobalTransform, &InheritedVisibility)>,
    mut gizmos: Gizmos<FractureGizmos>,
) {
    if !overlay.0 {
        return;
    }
    for (edges, transform, visibility) in &chunks {
        if !visibility.get() {
            continue;
        }
        for [a, b] in &edges.0 {
            gizmos.line(
                transform.transform_point(*a),
                transform.transform_point(*b),
                Color::srgb(1.0, 0.32, 0.055),
            );
        }
    }
}

pub fn fracture_edges<'a>(
    meshes: impl IntoIterator<Item = &'a Mesh>,
) -> Result<FractureEdges, String> {
    let mut positions = Vec::new();
    let mut buckets: HashMap<IVec3, Vec<usize>> = HashMap::new();
    let mut triangles = Vec::new();
    for mesh in meshes {
        if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
            return Err("fracture debug mesh must contain triangles".into());
        }
        let Some(VertexAttributeValues::Float32x3(vertices)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            return Err("fracture debug mesh has no float3 positions".into());
        };
        let welded: Vec<_> = vertices
            .iter()
            .map(|position| weld(Vec3::from(*position), &mut positions, &mut buckets))
            .collect();
        let indices: Vec<usize> = match mesh.indices() {
            Some(Indices::U16(indices)) => indices.iter().map(|&i| usize::from(i)).collect(),
            Some(Indices::U32(indices)) => indices.iter().map(|&i| i as usize).collect(),
            None => (0..vertices.len()).collect(),
        };
        if !indices.len().is_multiple_of(3) {
            return Err("fracture debug mesh has an incomplete triangle".into());
        }
        for triangle in indices.as_chunks::<3>().0 {
            let [Some(&a), Some(&b), Some(&c)] = [
                welded.get(triangle[0]),
                welded.get(triangle[1]),
                welded.get(triangle[2]),
            ] else {
                return Err("fracture debug mesh has an invalid vertex index".into());
            };
            let cross = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
            if cross.length_squared() > 1e-16 {
                triangles.push(([a, b, c], cross.normalize()));
            }
        }
    }

    let mut adjacency: HashMap<(usize, usize), Vec<Vec3>> = HashMap::new();
    for ([a, b, c], normal) in triangles {
        for (start, end) in [(a, b), (b, c), (c, a)] {
            // Material-region splits can introduce T-junctions. Split the debug
            // segments there as well so coplanar edges cancel across primitives.
            let origin = positions[start];
            let direction = positions[end] - origin;
            let length_squared = direction.length_squared();
            let mut points = vec![(0.0, start), (1.0, end)];
            for (id, position) in positions.iter().enumerate() {
                if id == start || id == end {
                    continue;
                }
                let t = (*position - origin).dot(direction) / length_squared;
                if t > 0.0
                    && t < 1.0
                    && position.distance_squared(origin + direction * t)
                        <= WELD_TOLERANCE * WELD_TOLERANCE
                {
                    points.push((t, id));
                }
            }
            points.sort_by(|a, b| a.0.total_cmp(&b.0));
            for pair in points.windows(2) {
                let a = pair[0].1;
                let b = pair[1].1;
                adjacency
                    .entry((a.min(b), a.max(b)))
                    .or_default()
                    .push(normal);
            }
        }
    }

    let mut edges: Vec<_> = adjacency
        .into_iter()
        .filter_map(|((a, b), normals)| {
            let coplanar = normals.len() >= 2
                && normals
                    .iter()
                    .skip(1)
                    .all(|normal| normals[0].dot(*normal) >= COPLANAR_COSINE);
            (!coplanar).then_some([positions[a], positions[b]])
        })
        .collect();
    edges.sort_by(|a, b| {
        a.iter()
            .flat_map(|point| point.to_array())
            .zip(b.iter().flat_map(|point| point.to_array()))
            .map(|(a, b)| a.total_cmp(&b))
            .find(|ordering| !ordering.is_eq())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(FractureEdges(edges))
}

fn weld(
    position: Vec3,
    positions: &mut Vec<Vec3>,
    buckets: &mut HashMap<IVec3, Vec<usize>>,
) -> usize {
    let cell = (position / WELD_TOLERANCE).floor().as_ivec3();
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if let Some(candidates) = buckets.get(&(cell + IVec3::new(x, y, z))) {
                    for &id in candidates {
                        if positions[id].distance_squared(position)
                            <= WELD_TOLERANCE * WELD_TOLERANCE
                        {
                            return id;
                        }
                    }
                }
            }
        }
    }
    let id = positions.len();
    positions.push(position);
    buckets.entry(cell).or_default().push(id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;

    fn triangle(vertices: [[f32; 3]; 3]) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices.to_vec())
    }

    #[test]
    fn zero_toggles_on_press_edges_for_both_keyboard_locations() {
        let mut app = App::new();
        app.init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<DebugOverlay>()
            .add_systems(Update, toggle_overlay);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Digit0);
        app.update();
        assert!(app.world().resource::<DebugOverlay>().0);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.update();
        assert!(app.world().resource::<DebugOverlay>().0);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Numpad0);
        app.update();
        assert!(!app.world().resource::<DebugOverlay>().0);
    }

    #[test]
    fn welds_material_primitives_and_removes_the_square_diagonal() {
        let a = triangle([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]]);
        let b = triangle([[0.000003, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]);
        let edges = fracture_edges([&a, &b]).unwrap();
        assert_eq!(edges.0.len(), 4);
    }

    #[test]
    fn preserves_real_folds_and_open_boundaries() {
        let a = triangle([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        let b = triangle([[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, 0.0, 1.0]]);
        assert_eq!(fracture_edges([&a, &b]).unwrap().0.len(), 5);
    }

    #[test]
    fn removes_coplanar_edges_at_material_t_junctions() {
        let a = triangle([[0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [1.0, 1.0, 0.0]]);
        let b = triangle([[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [1.0, -1.0, 0.0]]);
        let c = triangle([[2.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, -1.0, 0.0]]);
        let edges = fracture_edges([&a, &b, &c]).unwrap();
        assert_eq!(edges.0.len(), 4);
    }
}

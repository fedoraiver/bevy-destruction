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

use super::*;
use crate::asset::{AnchorRecord, BondRecord};

fn fixture(positions: &[V3], edges: &[(usize, usize)], anchors: &[usize]) -> Manifest {
    let mut manifest: Manifest = serde_json::from_str(include_str!(
        "../assets/buildings/rectangle.destruction.json"
    ))
    .unwrap();
    let template = manifest.chunks[0].clone();
    manifest.chunks = positions
        .iter()
        .enumerate()
        .map(|(i, position)| {
            let mut record = template.clone();
            record.id = format!("node-{i:03}");
            record.position = position.map(|value| value as f32);
            record.rotation = [0.0, 0.0, 0.0, 1.0];
            record.center_of_mass = [0.0; 3];
            record.mass = 10.0;
            record.material = "concrete".into();
            record
        })
        .collect();
    let template = manifest.bonds[0].clone();
    manifest.bonds = edges
        .iter()
        .enumerate()
        .map(|(i, &(a, b))| {
            let delta = sub3(positions[b], positions[a]);
            let normal = scale3(delta, 1.0 / norm3(delta));
            let q = scale_m3(subm3(identity3(), outer3(normal, normal)), 1.0 / 12.0);
            BondRecord {
                id: format!("bond-{i:03}"),
                a: manifest.chunks[a].id.clone(),
                b: manifest.chunks[b].id.clone(),
                center: scale3(add3(positions[a], positions[b]), 0.5).map(|value| value as f32),
                normal: normal.map(|value| value as f32),
                area: 1.0,
                area_covariance: q.map(|row| row.map(|value| value as f32)),
                ..template.clone()
            }
        })
        .collect();
    manifest.anchors = anchors
        .iter()
        .map(|&index| AnchorRecord {
            chunk: manifest.chunks[index].id.clone(),
            kind: "fixed".into(),
            position: manifest.chunks[index].position,
        })
        .collect();
    manifest
}

fn ids(manifest: &Manifest) -> Vec<String> {
    manifest
        .chunks
        .iter()
        .map(|chunk| chunk.id.clone())
        .collect()
}

fn bonds(manifest: &Manifest) -> BTreeMap<String, bool> {
    manifest
        .bonds
        .iter()
        .map(|bond| (bond.id.clone(), true))
        .collect()
}

fn close(actual: f64, expected: f64, relative: f64) {
    assert!(
        (actual - expected).abs() <= relative * expected.abs().max(1.0),
        "actual={actual:e}, expected={expected:e}"
    );
}

fn load(manifest: &Manifest, index: usize, force: V3, moment: V3) -> BTreeMap<String, NodeLoad> {
    BTreeMap::from([(
        manifest.chunks[index].id.clone(),
        NodeLoad { force, moment },
    )])
}

fn solve_fixture(manifest: &Manifest, loads: &BTreeMap<String, NodeLoad>) -> StressSolution {
    solve(
        manifest,
        &ids(manifest),
        &bonds(manifest),
        Vec3::ZERO,
        loads,
        &SolverSettings::default(),
        None,
    )
    .unwrap()
}

#[test]
fn cantilever_preserves_force_and_moment_equilibrium_as_lever_arm_grows() {
    for length in [1.0, 4.0] {
        let manifest = fixture(&[[0.0; 3], [length, 0.0, 0.0]], &[(0, 1)], &[0]);
        let result = solve_fixture(&manifest, &load(&manifest, 1, [0.0, -100.0, 0.0], [0.0; 3]));
        let stress = &result.bonds["bond-000"];
        close(stress.force[1], -100.0, 1e-10);
        close(stress.moment[2], -50.0 * length, 1e-10);
        let reaction = &result.reactions["node-000"];
        close(reaction.force[1], 100.0, 1e-10);
        close(reaction.moment[2], 100.0 * length, 1e-10);
        assert!(result.displacements["node-001"][1] < 0.0);
        assert!(result.relative_residual < 1e-10);
    }
}

#[test]
fn rectangular_section_recovers_axial_bending_and_torsional_stress_at_vertices() {
    let mut manifest = fixture(&[[0.0; 3], [1.0, 0.0, 0.0]], &[(0, 1)], &[0]);
    manifest.bonds[0].area_covariance[1][1] = 2.0 / 3.0;
    manifest.bonds[0].area_covariance[2][2] = 1.0 / 6.0;
    manifest.bonds[0].area = 2.0;
    let vertices = BTreeMap::from([(
        "bond-000".into(),
        vec![
            [0.5, -1.0, -0.5],
            [0.5, -1.0, 0.5],
            [0.5, 1.0, -0.5],
            [0.5, 1.0, 0.5],
        ],
    )]);
    let loads = load(&manifest, 1, [20.0, 0.0, 0.0], [4.0, 0.0, 8.0]);
    let result = solve(
        &manifest,
        &ids(&manifest),
        &bonds(&manifest),
        Vec3::ZERO,
        &loads,
        &SolverSettings::default(),
        Some(&vertices),
    )
    .unwrap();
    let stress = &result.bonds["bond-000"];
    close(stress.tension, 22.0, 1e-7);
    close(stress.compression, 2.0, 1e-6);
    close(
        stress.shear,
        4.0 / (2.0 / 3.0 + 1.0 / 6.0) * 1.25_f64.sqrt(),
        1e-7,
    );
    assert!(stress.section_exact);
    assert!(!solve_fixture(&manifest, &loads).bonds["bond-000"].section_exact);
}

#[test]
fn symmetric_and_unequal_stiffness_paths_share_load_and_redistribute_after_removal() {
    let mut manifest = fixture(
        &[[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0; 3]],
        &[(0, 2), (1, 2)],
        &[0, 1],
    );
    manifest
        .materials
        .get_mut("concrete")
        .unwrap()
        .young_modulus = 3e6;
    let loads = load(&manifest, 2, [100.0, 0.0, 0.0], [0.0; 3]);
    let symmetric = solve_fixture(&manifest, &loads);
    close(symmetric.reactions["node-000"].force[0], -50.0, 1e-10);
    close(symmetric.reactions["node-001"].force[0], -50.0, 1e-10);

    let mut soft = manifest.materials["concrete"].clone();
    soft.young_modulus = 1e6;
    manifest.materials.insert("soft".into(), soft);
    manifest.chunks[0].material = "soft".into();
    let unequal = solve_fixture(&manifest, &loads);
    close(unequal.reactions["node-000"].force[0], -100.0 / 3.0, 1e-10);
    close(unequal.reactions["node-001"].force[0], -200.0 / 3.0, 1e-10);

    let mut active = bonds(&manifest);
    active.insert("bond-000".into(), false);
    let remaining = solve(
        &manifest,
        &ids(&manifest),
        &active,
        Vec3::ZERO,
        &loads,
        &SolverSettings::default(),
        None,
    )
    .unwrap();
    close(remaining.reactions["node-000"].force[0], 0.0, 1e-10);
    close(remaining.reactions["node-001"].force[0], -100.0, 1e-10);
}

fn grid() -> Manifest {
    let positions = (0..3)
        .flat_map(|x| (0..3).map(move |z| [x as f64, 1.0, z as f64]))
        .collect::<Vec<_>>();
    let mut edges = Vec::new();
    for x in 0..3 {
        for z in 0..3 {
            let index = x * 3 + z;
            if x < 2 {
                edges.push((index, index + 3));
            }
            if z < 2 {
                edges.push((index, index + 1));
            }
        }
    }
    fixture(&positions, &edges, &[0, 1, 2])
}

fn dense_reference(model: &Model) -> Vec<f64> {
    let n = model.load.len();
    let mut matrix = vec![vec![0.0; n]; n];
    for (i, row) in model.rows.iter().enumerate() {
        for (&j, block) in row {
            for r in 0..6 {
                for c in 0..6 {
                    matrix[i * 6 + r][j * 6 + c] += block[r][c];
                }
            }
        }
    }
    let mut factor = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let value = matrix[i][j] - (0..j).map(|k| factor[i][k] * factor[j][k]).sum::<f64>();
            factor[i][j] = if i == j {
                assert!(value > 0.0);
                value.sqrt()
            } else {
                value / factor[j][j]
            };
        }
    }
    let mut x = vec![0.0; n];
    for i in 0..n {
        x[i] = (model.load[i] - (0..i).map(|j| factor[i][j] * x[j]).sum::<f64>()) / factor[i][i];
    }
    for i in (0..n).rev() {
        x[i] = (x[i] - (i + 1..n).map(|j| factor[j][i] * x[j]).sum::<f64>()) / factor[i][i];
    }
    x
}

#[test]
fn closed_network_matches_dense_reference_and_redistributes_after_path_removal() {
    let manifest = grid();
    let gravity = Vec3::new(0.0, -9.81, 0.0);
    let mut active = bonds(&manifest);
    let model = assemble(
        &manifest,
        &ids(&manifest),
        &active,
        gravity,
        &BTreeMap::new(),
    )
    .unwrap();
    let dense = dense_reference(&model);
    let (iterative, _, residual) = pcg(&model, &SolverSettings::default()).unwrap();
    assert!(residual <= 1e-8);
    for (&actual, &expected) in iterative.iter().zip(&dense) {
        close(actual, expected, 1e-12);
    }
    let first = solve(
        &manifest,
        &ids(&manifest),
        &active,
        gravity,
        &BTreeMap::new(),
        &SolverSettings::default(),
        None,
    )
    .unwrap();
    active.insert(manifest.bonds[0].id.clone(), false);
    let second = solve(
        &manifest,
        &ids(&manifest),
        &active,
        gravity,
        &BTreeMap::new(),
        &SolverSettings::default(),
        None,
    )
    .unwrap();
    assert!(!second.bonds.contains_key(&manifest.bonds[0].id));
    assert!(
        second
            .bonds
            .iter()
            .any(|(id, stress)| norm3(sub3(stress.force, first.bonds[id].force)) > 0.1)
    );
    for result in [&first, &second] {
        close(
            result.reactions.values().map(|load| load.force[1]).sum(),
            9.0 * 10.0 * 9.81,
            1e-7,
        );
    }
}

#[test]
fn assembly_is_objective_and_independent_of_input_order() {
    let mut manifest = grid();
    let model = assemble(
        &manifest,
        &ids(&manifest),
        &bonds(&manifest),
        Vec3::ZERO,
        &BTreeMap::new(),
    )
    .unwrap();
    let translation = [0.3, -0.7, 1.2];
    let rotation = [0.5, -0.2, 0.7];
    for element in &model.elements {
        let q = |index: usize| {
            let translation = add3(translation, cross3(rotation, model.nodes[index].center));
            [
                translation[0],
                translation[1],
                translation[2],
                rotation[0] * model.length_scale,
                rotation[1] * model.length_scale,
                rotation[2] * model.length_scale,
            ]
        };
        let deformation = add6(
            mul6(element.ja, q(element.a)),
            mul6(element.jb, q(element.b)),
        );
        assert!(
            deformation
                .into_iter()
                .all(|component| component.abs() < 1e-14)
        );
    }
    let load = load(&manifest, 8, [13.0, -27.0, 8.0], [2.0, -3.0, 4.0]);
    let first = solve_fixture(&manifest, &load);
    manifest.chunks.reverse();
    manifest.bonds.reverse();
    let mut shuffled = ids(&manifest);
    shuffled.rotate_left(3);
    let second = solve(
        &manifest,
        &shuffled,
        &bonds(&manifest),
        Vec3::ZERO,
        &load,
        &SolverSettings::default(),
        None,
    )
    .unwrap();
    for (id, stress) in first.bonds {
        for axis in 0..3 {
            close(second.bonds[&id].force[axis], stress.force[axis], 1e-9);
        }
    }
}

#[test]
fn support_loss_invalid_geometry_and_budget_exhaustion_are_explicit_failures() {
    let mut manifest = grid();
    let limited = SolverSettings {
        max_iterations: 1,
        ..Default::default()
    };
    assert!(matches!(
        solve(
            &manifest,
            &ids(&manifest),
            &bonds(&manifest),
            Vec3::NEG_Y,
            &BTreeMap::new(),
            &limited,
            None
        ),
        Err(SolveError::NonConvergence { .. })
    ));
    manifest.bonds[0].area_covariance = [[0.0; 3]; 3];
    assert!(matches!(
        solve(
            &manifest,
            &ids(&manifest),
            &bonds(&manifest),
            Vec3::NEG_Y,
            &BTreeMap::new(),
            &SolverSettings::default(),
            None
        ),
        Err(SolveError::InvalidModel(_))
    ));
    manifest.anchors.clear();
    assert!(matches!(
        solve(
            &manifest,
            &ids(&manifest),
            &bonds(&manifest),
            Vec3::NEG_Y,
            &BTreeMap::new(),
            &SolverSettings::default(),
            None
        ),
        Err(SolveError::Underconstrained(_))
    ));
}

#[test]
fn bundled_graphs_converge_with_balanced_self_weight_reactions() {
    for file in [
        "highrise_slab",
        "highrise_l",
        "bridge",
        "rectangle",
        "l_shape",
        "courtyard",
    ] {
        let path = format!(
            "{}/assets/buildings/{file}.destruction.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let contents = std::fs::read_to_string(path).unwrap();
        let manifest: Manifest = serde_json::from_str(&contents).unwrap();
        let start = std::time::Instant::now();
        let result = solve(
            &manifest,
            &ids(&manifest),
            &bonds(&manifest),
            Vec3::new(0.0, -9.81, 0.0),
            &BTreeMap::new(),
            &SolverSettings::default(),
            None,
        )
        .unwrap_or_else(|error| panic!("{file}: {error}"));
        println!(
            "Stress {file}: {} nodes, {} iterations, residual={:e}, max_util={:e}, elapsed={:?}",
            manifest.chunks.len(),
            result.iterations,
            result.relative_residual,
            result
                .bonds
                .values()
                .map(|stress| stress.utilization)
                .fold(0.0_f64, f64::max),
            start.elapsed()
        );
        let total_mass: f64 = manifest.chunks.iter().map(|chunk| chunk.mass as f64).sum();
        close(
            result
                .reactions
                .values()
                .map(|reaction| reaction.force[1])
                .sum(),
            total_mass * 9.81,
            1e-6,
        );
        assert!(
            result
                .bonds
                .values()
                .all(|stress| stress.utilization.is_finite())
        );
    }
}

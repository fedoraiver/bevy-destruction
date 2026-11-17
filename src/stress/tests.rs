use super::*;
use crate::asset::{AnchorRecord, BondPatchRecord, BondRecord};

fn fixture(positions: &[V3], edges: &[(usize, usize)], anchors: &[usize]) -> Manifest {
    let mut manifest: Manifest = serde_json::from_str(include_str!(
        "../../assets/buildings/rectangle.destruction.json"
    ))
    .unwrap();
    manifest.schema_version = 1;
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
                patches: Vec::new(),
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

#[test]
fn balanced_dynamic_loads_are_independent_of_numerical_gauge() {
    let manifest = fixture(&[[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]], &[(0, 1)], &[0, 1]);
    let anchors = manifest
        .anchors
        .iter()
        .map(|anchor| anchor.chunk.clone())
        .collect::<Vec<_>>();
    let loads = BTreeMap::from([
        (
            "node-000".into(),
            NodeLoad {
                force: [100.0, 20.0, -30.0],
                moment: [0.0, 30.0, 20.0],
            },
        ),
        (
            "node-001".into(),
            NodeLoad {
                force: [-100.0, -20.0, 30.0],
                moment: [0.0, 30.0, 20.0],
            },
        ),
    ]);
    let first = solve_with_gauge(
        &manifest,
        &ids(&manifest),
        &bonds(&manifest),
        &loads,
        &SolverSettings::default(),
        None,
        "node-000",
    )
    .unwrap();
    let second = solve_with_gauge(
        &manifest,
        &ids(&manifest),
        &bonds(&manifest),
        &loads,
        &SolverSettings::default(),
        None,
        "node-001",
    )
    .unwrap();
    for result in [&first, &second] {
        close(result.bonds["bond-000"].force[0], -100.0, 1e-10);
        for reaction in result.reactions.values() {
            assert!(norm3(reaction.force) < 1e-8);
            assert!(norm3(reaction.moment) < 1e-8);
        }
    }
    for axis in 0..3 {
        close(
            first.bonds["bond-000"].force[axis],
            second.bonds["bond-000"].force[axis],
            1e-10,
        );
        close(
            first.bonds["bond-000"].moment[axis],
            second.bonds["bond-000"].moment[axis],
            1e-10,
        );
    }
    assert_eq!(
        manifest
            .anchors
            .iter()
            .map(|anchor| anchor.chunk.clone())
            .collect::<Vec<_>>(),
        anchors
    );

    let unbalanced_force = load(&manifest, 0, [1.0, 0.0, 0.0], [0.0; 3]);
    assert!(matches!(
        solve_with_gauge(
            &manifest,
            &ids(&manifest),
            &bonds(&manifest),
            &unbalanced_force,
            &SolverSettings::default(),
            None,
            "node-000"
        ),
        Err(SolveError::InvalidModel(_))
    ));
    let unbalanced_moment = BTreeMap::from([
        (
            "node-000".into(),
            NodeLoad {
                force: [0.0, 1.0, 0.0],
                ..Default::default()
            },
        ),
        (
            "node-001".into(),
            NodeLoad {
                force: [0.0, -1.0, 0.0],
                ..Default::default()
            },
        ),
    ]);
    assert!(matches!(
        solve_with_gauge(
            &manifest,
            &ids(&manifest),
            &bonds(&manifest),
            &unbalanced_moment,
            &SolverSettings::default(),
            None,
            "node-000"
        ),
        Err(SolveError::InvalidModel(_))
    ));
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
    for (i, offsets) in model.row_offsets.windows(2).enumerate() {
        for block in &model.blocks[offsets[0]..offsets[1]] {
            let j = block.column;
            for r in 0..6 {
                for c in 0..6 {
                    matrix[i * 6 + r][j * 6 + c] += block.values[r][c];
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
    let (refined, _, residual) = refine(
        &model,
        &SolverSettings::default(),
        vec![0.0; model.load.len()],
        0,
        dot(&model.load, &model.load).sqrt(),
    )
    .unwrap();
    assert!(residual <= 1e-8);
    for (&actual, &expected) in refined.iter().zip(&dense) {
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
fn damaged_rectangle_with_one_remaining_support_interface_converges() {
    let manifest: Manifest = serde_json::from_str(include_str!(
        "../../tests/fixtures/rectangle-v1.destruction.json"
    ))
    .unwrap();
    let centers = manifest
        .chunks
        .iter()
        .map(|chunk| (chunk.id.as_str(), chunk.position[1]))
        .collect::<BTreeMap<_, _>>();
    let active = manifest
        .bonds
        .iter()
        .map(|bond| {
            let crosses_cut = (centers[bond.a.as_str()] < 1.0) != (centers[bond.b.as_str()] < 1.0);
            (
                bond.id.clone(),
                !crosses_cut || bond.id == "bond_da333b7d157787cb4b2d6145",
            )
        })
        .collect();
    let solution = solve(
        &manifest,
        &ids(&manifest),
        &active,
        Vec3::new(0.0, -9.81, 0.0),
        &BTreeMap::new(),
        &SolverSettings::default(),
        None,
    )
    .unwrap();
    assert!(solution.relative_residual <= 1e-8);
    assert_recovered_equilibrium(
        &assemble(
            &manifest,
            &ids(&manifest),
            &active,
            Vec3::new(0.0, -9.81, 0.0),
            &BTreeMap::new(),
        )
        .unwrap(),
        &solution,
    );
    let total_weight = manifest
        .chunks
        .iter()
        .map(|chunk| f64::from(chunk.mass))
        .sum::<f64>()
        * 9.81;
    close(
        solution
            .reactions
            .values()
            .map(|reaction| reaction.force[1])
            .sum(),
        total_weight,
        1e-6,
    );
    assert!(
        solution
            .bonds
            .values()
            .all(|bond| bond.utilization.is_finite())
    );
}

fn assert_recovered_equilibrium(model: &Model, solution: &StressSolution) {
    let mut internal = vec![NodeLoad::default(); model.nodes.len()];
    let mut seen = BTreeSet::new();
    for element in &model.elements {
        if !seen.insert(&element.id) {
            continue;
        }
        let stress = &solution.bonds[&element.id];
        assert!(stress.utilization.is_finite());
        for (index, sign) in [(element.a, -1.0), (element.b, 1.0)] {
            let force = scale3(stress.force, sign);
            let moment = add3(
                scale3(stress.moment, sign),
                cross3(sub3(element.bond_center, model.nodes[index].center), force),
            );
            internal[index].force = add3(internal[index].force, force);
            internal[index].moment = add3(internal[index].moment, moment);
        }
    }
    let mut squared = 0.0;
    for (node, internal) in model.nodes.iter().zip(internal) {
        if !node.fixed {
            squared += norm3(sub3(internal.force, node.load.force)).powi(2);
            squared +=
                (norm3(sub3(internal.moment, node.load.moment)) / model.length_scale).powi(2);
        }
    }
    assert!(solution.relative_residual <= 1e-8);
    assert!(squared.sqrt() / dot(&model.load, &model.load).sqrt() <= 1e-8);
    let mut force = [0.0; 3];
    let mut moment = [0.0; 3];
    let mut force_scale = 0.0;
    let mut moment_scale = 0.0;
    for node in &model.nodes {
        let reaction = solution
            .reactions
            .get(&node.id)
            .copied()
            .unwrap_or_default();
        let load_force = add3(node.load.force, reaction.force);
        force = add3(force, load_force);
        moment = add3(
            moment,
            add3(
                add3(node.load.moment, reaction.moment),
                cross3(node.center, load_force),
            ),
        );
        force_scale += norm3(node.load.force);
        moment_scale += norm3(node.load.moment) + norm3(node.center) * norm3(node.load.force);
    }
    assert!(norm3(force) <= 1e-7 * force_scale);
    assert!(norm3(moment) <= 1e-7 * moment_scale);
}

#[test]
fn narrow_wall_jamb_transfers_the_upper_building_weight_and_moment() {
    let manifest: Manifest = serde_json::from_str(include_str!(
        "../../tests/fixtures/rectangle-v1.destruction.json"
    ))
    .unwrap();
    let gravity = Vec3::new(0.0, -9.81, 0.0);
    for narrow_jamb in [true, false] {
        let chunks = manifest
            .chunks
            .iter()
            .filter(|chunk| {
                !(chunk.part_id.contains("-l00")
                    && if narrow_jamb {
                        chunk.part_id != "wall-inner-0-l00-p0000-z00"
                    } else {
                        chunk.position[0] < 4.04
                    })
            })
            .map(|chunk| chunk.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(chunks.len(), if narrow_jamb { 194 } else { 217 });
        let alive = chunks.iter().collect::<BTreeSet<_>>();
        let active = manifest
            .bonds
            .iter()
            .map(|bond| {
                (
                    bond.id.clone(),
                    alive.contains(&bond.a) && alive.contains(&bond.b),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let model = assemble(&manifest, &chunks, &active, gravity, &BTreeMap::new()).unwrap();
        let solution = solve(
            &manifest,
            &chunks,
            &active,
            gravity,
            &BTreeMap::new(),
            &SolverSettings::default(),
            None,
        )
        .unwrap();
        assert_recovered_equilibrium(&model, &solution);
        if !narrow_jamb {
            assert!(
                solution
                    .bonds
                    .values()
                    .all(|stress| stress.utilization < 1.0)
            );
            continue;
        }
        let bridge = model
            .elements
            .iter()
            .find(|element| element.id == "bond_afddaa5aa6e3105729db1b11")
            .unwrap();
        let mut upper = BTreeSet::from([bridge.b]);
        loop {
            let before = upper.len();
            for element in &model.elements {
                if element.id != bridge.id
                    && (upper.contains(&element.a) || upper.contains(&element.b))
                {
                    upper.insert(element.a);
                    upper.insert(element.b);
                }
            }
            if upper.len() == before {
                break;
            }
        }
        assert_eq!(upper.len(), 150);
        assert!(!upper.iter().any(|&index| model.nodes[index].fixed));
        let mut force = [0.0; 3];
        let mut moment = [0.0; 3];
        for &index in &upper {
            let node = &model.nodes[index];
            force = add3(force, node.load.force);
            moment = add3(
                moment,
                cross3(sub3(node.center, bridge.center), node.load.force),
            );
        }
        let stress = &solution.bonds[&bridge.id];
        assert!(norm3(sub3(stress.force, force)) <= 1e-7 * norm3(force));
        assert!(norm3(sub3(stress.moment, moment)) <= 1e-7 * norm3(moment));
        assert!(stress.compression >= norm3(force) / bridge.area * (1.0 - 1e-6));
        assert!(stress.utilization > 4.0);
    }
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

fn rectangular_patch(center: V3, normal: V3, half_width: f64, half_height: f64) -> BondPatchRecord {
    let tangent = [-normal[1], normal[0], 0.0];
    let bitangent = [0.0, 0.0, 1.0];
    let area = 4.0 * half_width * half_height;
    let q = addm3(
        scale_m3(outer3(tangent, tangent), area * half_width.powi(2) / 3.0),
        scale_m3(
            outer3(bitangent, bitangent),
            area * half_height.powi(2) / 3.0,
        ),
    );
    BondPatchRecord {
        area: area as f32,
        center: center.map(|v| v as f32),
        normal: normal.map(|v| v as f32),
        area_covariance: q.map(|row| row.map(|v| v as f32)),
        vertices: [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .into_iter()
            .map(|(u, v)| {
                add3(
                    center,
                    add3(
                        scale3(tangent, u * half_width),
                        scale3(bitangent, v * half_height),
                    ),
                )
                .map(|v| v as f32)
            })
            .collect(),
    }
}

fn set_interface_patches(bond: &mut BondRecord, patches: Vec<BondPatchRecord>) {
    let area = patches
        .iter()
        .map(|patch| f64::from(patch.area))
        .sum::<f64>();
    let center = scale3(
        patches.iter().fold([0.0; 3], |sum, patch| {
            add3(
                sum,
                scale3(patch.center.map(f64::from), f64::from(patch.area)),
            )
        }),
        1.0 / area,
    );
    let normal = patches.iter().fold([0.0; 3], |sum, patch| {
        add3(
            sum,
            scale3(patch.normal.map(f64::from), f64::from(patch.area)),
        )
    });
    let q = patches.iter().fold([[0.0; 3]; 3], |sum, patch| {
        let offset = sub3(patch.center.map(f64::from), center);
        addm3(
            sum,
            addm3(
                patch.area_covariance.map(|r| r.map(f64::from)),
                scale_m3(outer3(offset, offset), f64::from(patch.area)),
            ),
        )
    });
    bond.area = area as f32;
    bond.center = center.map(|v| v as f32);
    bond.normal = scale3(normal, 1.0 / norm3(normal)).map(|v| v as f32);
    bond.area_covariance = q.map(|row| row.map(|v| v as f32));
    bond.patches = patches;
}

#[test]
fn planar_patch_subdivision_preserves_stiffness_reactions_and_peak_stress() {
    let mut whole = fixture(&[[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]], &[(0, 1)], &[0]);
    set_interface_patches(
        &mut whole.bonds[0],
        vec![rectangular_patch([0.0; 3], [1.0, 0.0, 0.0], 1.0, 1.0)],
    );
    let mut divided = whole.clone();
    set_interface_patches(
        &mut divided.bonds[0],
        vec![
            rectangular_patch([0.0, -0.5, 0.0], [1.0, 0.0, 0.0], 0.5, 1.0),
            rectangular_patch([0.0, 0.5, 0.0], [1.0, 0.0, 0.0], 0.5, 1.0),
        ],
    );
    let mut legacy = whole.clone();
    let vertices = BTreeMap::from([(
        "bond-000".to_owned(),
        whole.bonds[0].patches[0]
            .vertices
            .iter()
            .map(|p| p.map(f64::from))
            .collect(),
    )]);
    legacy.bonds[0].patches.clear();
    let loads = load(&whole, 1, [100.0, 25.0, -30.0], [20.0, 40.0, -50.0]);
    let original = solve(
        &legacy,
        &ids(&legacy),
        &bonds(&legacy),
        Vec3::ZERO,
        &loads,
        &SolverSettings::default(),
        Some(&vertices),
    )
    .unwrap();
    for manifest in [&whole, &divided] {
        let model = assemble(
            manifest,
            &ids(manifest),
            &bonds(manifest),
            Vec3::ZERO,
            &loads,
        )
        .unwrap();
        let result = solve_fixture(manifest, &loads);
        assert_recovered_equilibrium(&model, &result);
        assert_eq!(result.bonds.len(), 1);
        for (actual, expected) in result.displacements["node-001"]
            .iter()
            .zip(original.displacements["node-001"])
        {
            assert!((actual - expected).abs() < 2e-7 * expected.abs().max(1e-12));
        }
        let a = &result.bonds["bond-000"];
        let b = &original.bonds["bond-000"];
        assert!(a.section_exact);
        for (actual, expected) in a
            .force
            .into_iter()
            .chain(a.moment)
            .chain([a.tension, a.compression, a.shear, a.utilization])
            .zip(b.force.into_iter().chain(b.moment).chain([
                b.tension,
                b.compression,
                b.shear,
                b.utilization,
            ]))
        {
            close(actual, expected, 2e-7);
        }
    }
}

#[test]
fn folded_interface_matches_independent_surface_integral_and_parent_equilibrium() {
    let mut manifest = fixture(&[[-1.0, 0.0, 0.0], [1.0, 0.0, 0.0]], &[(0, 1)], &[0]);
    let angle = std::f64::consts::PI / 6.0;
    let (sine, cosine) = angle.sin_cos();
    set_interface_patches(
        &mut manifest.bonds[0],
        vec![
            rectangular_patch(
                [0.5 * sine, -0.5 * cosine, 0.0],
                [cosine, sine, 0.0],
                0.5,
                1.0,
            ),
            rectangular_patch(
                [0.5 * sine, 0.5 * cosine, 0.0],
                [cosine, -sine, 0.0],
                0.5,
                1.0,
            ),
        ],
    );
    let loads = load(&manifest, 1, [100.0, 25.0, -30.0], [20.0, 40.0, -50.0]);
    let model = assemble(
        &manifest,
        &ids(&manifest),
        &bonds(&manifest),
        Vec3::ZERO,
        &loads,
    )
    .unwrap();
    let displacement = [0.001, -0.002, 0.003, 0.0003, -0.0004, 0.0007];
    let mut product = [0.0; 6];
    model.multiply(&displacement, &mut product);
    let energy = dot(&displacement, &product);
    let material = &manifest.materials["concrete"];
    let young = f64::from(material.young_modulus);
    let shear = shear_modulus(material);
    let translation = [displacement[0], displacement[1], displacement[2]];
    let rotation = scale3(
        [displacement[3], displacement[4], displacement[5]],
        1.0 / model.length_scale,
    );
    let mut integral = 0.0;
    for patch in &manifest.bonds[0].patches {
        let normal = patch.normal.map(f64::from);
        let tangent = [-normal[1], normal[0], 0.0];
        for u in [-1.0_f64 / 3.0_f64.sqrt(), 1.0 / 3.0_f64.sqrt()] {
            for v in [-1.0_f64 / 3.0_f64.sqrt(), 1.0 / 3.0_f64.sqrt()] {
                let point = add3(
                    patch.center.map(f64::from),
                    add3(scale3(tangent, u * 0.5), [0.0, 0.0, v]),
                );
                let delta = add3(translation, cross3(rotation, sub3(point, [1.0, 0.0, 0.0])));
                let normal_displacement = dot3(normal, delta);
                integral += f64::from(patch.area) / 4.0 / model.length_scale
                    * (young * normal_displacement.powi(2)
                        + shear * (dot3(delta, delta) - normal_displacement.powi(2)));
            }
        }
    }
    close(energy, integral, 2e-7);
    let result = solve_fixture(&manifest, &loads);
    assert_recovered_equilibrium(&model, &result);
    assert_eq!(result.bonds.len(), 1);
    assert!(result.bonds["bond-000"].section_exact);
    let bond = &result.bonds["bond-000"];
    let expected = loads["node-001"];
    let moment = add3(
        bond.moment,
        cross3(
            sub3(manifest.bonds[0].center.map(f64::from), [1.0, 0.0, 0.0]),
            bond.force,
        ),
    );
    for axis in 0..3 {
        close(bond.force[axis], expected.force[axis], 1e-8);
        close(moment[axis], expected.moment[axis], 1e-8);
    }
}

fn add6(a: V6, b: V6) -> V6 {
    std::array::from_fn(|i| a[i] + b[i])
}

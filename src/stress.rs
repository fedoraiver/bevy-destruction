use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::Vec3;

use crate::asset::{Manifest, PhysicalMaterial};

#[cfg(test)]
#[path = "stress_tests.rs"]
mod tests;

type V3 = [f64; 3];
type V6 = [f64; 6];
type M3 = [[f64; 3]; 3];
type M6 = [[f64; 6]; 6];

#[derive(Debug, Clone)]
pub struct SolverSettings {
    pub relative_tolerance: f64,
    pub max_iterations: usize,
}

impl Default for SolverSettings {
    fn default() -> Self {
        Self {
            relative_tolerance: 1e-8,
            max_iterations: 3000,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NodeLoad {
    pub force: V3,
    pub moment: V3,
}

#[derive(Debug, Clone)]
pub struct BondStress {
    /// Force and couple acting on endpoint a at the interface center.
    pub force: V3,
    pub moment: V3,
    pub tension: f64,
    pub compression: f64,
    pub shear: f64,
    pub utilization: f64,
    pub section_exact: bool,
}

#[derive(Debug, Clone)]
pub struct StressSolution {
    pub bonds: BTreeMap<String, BondStress>,
    /// Support actions at anchored chunk centers of mass.
    pub reactions: BTreeMap<String, NodeLoad>,
    pub displacements: BTreeMap<String, V6>,
    pub iterations: usize,
    pub relative_residual: f64,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SolveError {
    #[error("Invalid structural model: {0}")]
    InvalidModel(String),
    #[error("Chunks have no fixed support path: {0:?}")]
    Underconstrained(Vec<String>),
    #[error("Structural solver broke down at iteration {iteration}")]
    NumericalBreakdown { iteration: usize },
    #[error(
        "Structural solver did not converge in {iterations} iterations (relative residual {relative_residual:e})"
    )]
    NonConvergence {
        iterations: usize,
        relative_residual: f64,
    },
}

struct Node {
    id: String,
    center: V3,
    load: NodeLoad,
    fixed: bool,
    free: Option<usize>,
}

struct Element {
    id: String,
    a: usize,
    b: usize,
    center: V3,
    normal: V3,
    area: f64,
    q: M3,
    tangent: V3,
    bitangent: V3,
    inverse_q: [[f64; 2]; 2],
    max_radius: f64,
    stiffness: M6,
    ja: M6,
    jb: M6,
    tension_strength: f64,
    compression_strength: f64,
    shear_strength: f64,
}

struct Model {
    nodes: Vec<Node>,
    elements: Vec<Element>,
    row_offsets: Vec<usize>,
    blocks: Vec<StiffnessBlock>,
    diagonal_blocks: Vec<Option<usize>>,
    load: Vec<f64>,
    length_scale: f64,
}

struct StiffnessBlock {
    column: usize,
    values: M6,
}

#[allow(clippy::too_many_arguments)]
pub fn solve(
    manifest: &Manifest,
    chunks: &[String],
    active_bonds: &BTreeMap<String, bool>,
    gravity: Vec3,
    external_loads: &BTreeMap<String, NodeLoad>,
    settings: &SolverSettings,
    section_vertices: Option<&BTreeMap<String, Vec<V3>>>,
) -> Result<StressSolution, SolveError> {
    validate_sections(section_vertices)?;
    let model = assemble(manifest, chunks, active_bonds, gravity, external_loads)?;
    let (x, iterations, relative_residual) = pcg(&model, settings)?;
    Ok(model.recover(&x, iterations, relative_residual, section_vertices))
}

/// Solves balanced free-body loads; the gauge removes rigid modes, not a physical support.
/// Asset anchors are ignored and remain unchanged.
pub fn solve_with_gauge(
    manifest: &Manifest,
    chunks: &[String],
    active_bonds: &BTreeMap<String, bool>,
    external_loads: &BTreeMap<String, NodeLoad>,
    settings: &SolverSettings,
    section_vertices: Option<&BTreeMap<String, Vec<V3>>>,
    gauge: &str,
) -> Result<StressSolution, SolveError> {
    validate_sections(section_vertices)?;
    let model = assemble_with_gauge(
        manifest,
        chunks,
        active_bonds,
        Vec3::ZERO,
        external_loads,
        Some(gauge),
    )?;
    let reference = scale3(
        model
            .nodes
            .iter()
            .fold([0.0; 3], |sum, node| add3(sum, node.center)),
        1.0 / model.nodes.len() as f64,
    );
    let mut force = [0.0; 3];
    let mut moment = [0.0; 3];
    let mut force_scale = 0.0;
    let mut moment_scale = 0.0;
    for node in &model.nodes {
        let offset = sub3(node.center, reference);
        force = add3(force, node.load.force);
        moment = add3(
            moment,
            add3(cross3(offset, node.load.force), node.load.moment),
        );
        force_scale += norm3(node.load.force);
        moment_scale += norm3(offset) * norm3(node.load.force) + norm3(node.load.moment);
    }
    if norm3(force) > 1e-6 * force_scale || norm3(moment) > 1e-6 * moment_scale {
        return invalid("Numerical gauge loads must balance force and moment");
    }
    let (x, iterations, relative_residual) = pcg(&model, settings)?;
    Ok(model.recover(&x, iterations, relative_residual, section_vertices))
}

fn validate_sections(
    section_vertices: Option<&BTreeMap<String, Vec<V3>>>,
) -> Result<(), SolveError> {
    if section_vertices.is_some_and(|sections| {
        sections
            .values()
            .flatten()
            .flatten()
            .any(|value| !value.is_finite())
    }) {
        return invalid("Non-finite interface section vertices");
    }
    Ok(())
}

fn assemble(
    manifest: &Manifest,
    chunks: &[String],
    active_bonds: &BTreeMap<String, bool>,
    gravity: Vec3,
    external_loads: &BTreeMap<String, NodeLoad>,
) -> Result<Model, SolveError> {
    assemble_with_gauge(
        manifest,
        chunks,
        active_bonds,
        gravity,
        external_loads,
        None,
    )
}

fn assemble_with_gauge(
    manifest: &Manifest,
    chunks: &[String],
    active_bonds: &BTreeMap<String, bool>,
    gravity: Vec3,
    external_loads: &BTreeMap<String, NodeLoad>,
    gauge: Option<&str>,
) -> Result<Model, SolveError> {
    if !gravity.is_finite() {
        return invalid("Gravity is not finite");
    }
    let records = manifest
        .chunks
        .iter()
        .map(|record| (record.id.as_str(), record))
        .collect::<BTreeMap<_, _>>();
    if gauge.is_some_and(|id| !chunks.iter().any(|chunk| chunk == id)) {
        return invalid("Numerical gauge must identify a solved chunk");
    }
    let anchors = if let Some(gauge) = gauge {
        BTreeSet::from([gauge])
    } else {
        manifest
            .anchors
            .iter()
            .filter(|anchor| anchor.kind == "fixed")
            .map(|anchor| anchor.chunk.as_str())
            .collect::<BTreeSet<_>>()
    };
    let mut nodes = Vec::new();
    let mut indices = BTreeMap::new();
    let mut free_count = 0;
    for id in chunks.iter().collect::<BTreeSet<_>>() {
        let Some(record) = records.get(id.as_str()) else {
            return invalid(&format!("Unknown chunk {id}"));
        };
        if !record.mass.is_finite() || record.mass <= 0.0 {
            return invalid(&format!("Invalid mass for {id}"));
        }
        let center = record
            .transform()
            .transform_point(Vec3::from(record.center_of_mass))
            .as_dvec3()
            .to_array();
        let fixed = anchors.contains(id.as_str());
        let mut load = external_loads.get(id).copied().unwrap_or_default();
        load.force = add3(
            load.force,
            scale3(gravity.as_dvec3().to_array(), record.mass as f64),
        );
        if !center
            .into_iter()
            .chain(load.force)
            .chain(load.moment)
            .all(f64::is_finite)
        {
            return invalid(&format!("Non-finite geometry or load for {id}"));
        }
        indices.insert(id.as_str(), nodes.len());
        nodes.push(Node {
            id: id.clone(),
            center,
            load,
            fixed,
            free: if fixed {
                None
            } else {
                let index = free_count;
                free_count += 1;
                Some(index)
            },
        });
    }
    if nodes.is_empty() {
        return invalid("No chunks to solve");
    }
    let mut bonds = manifest
        .bonds
        .iter()
        .filter(|bond| active_bonds.get(&bond.id) == Some(&true))
        .filter_map(|bond| {
            Some((
                bond,
                *indices.get(bond.a.as_str())?,
                *indices.get(bond.b.as_str())?,
            ))
        })
        .collect::<Vec<_>>();
    bonds.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    let mut adjacent = vec![Vec::new(); nodes.len()];
    for &(_, a, b) in &bonds {
        adjacent[a].push(b);
        adjacent[b].push(a);
    }
    let mut reached = vec![false; nodes.len()];
    let mut pending = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        if node.fixed {
            reached[i] = true;
            pending.push(i);
        }
    }
    while let Some(i) = pending.pop() {
        for &j in &adjacent[i] {
            if !reached[j] {
                reached[j] = true;
                pending.push(j);
            }
        }
    }
    if reached.iter().any(|reached| !reached) {
        return Err(SolveError::Underconstrained(
            nodes
                .iter()
                .zip(reached)
                .filter(|(_, reached)| !reached)
                .map(|(node, _)| node.id.clone())
                .collect(),
        ));
    }
    let mut lengths = Vec::new();
    for &(bond, a, b) in &bonds {
        let center = bond.center.map(f64::from);
        let length = norm3(sub3(center, nodes[a].center)) + norm3(sub3(center, nodes[b].center));
        if !length.is_finite() || length <= 1e-10 {
            return invalid(&format!("Degenerate equivalent length for {}", bond.id));
        }
        lengths.push(length);
    }
    let length_scale = if lengths.is_empty() {
        1.0
    } else {
        lengths.iter().sum::<f64>() / lengths.len() as f64
    };
    let mut elements = Vec::new();
    for ((bond, a, b), length) in bonds.into_iter().zip(lengths) {
        let material_a = &manifest.materials[&records[bond.a.as_str()].material];
        let material_b = &manifest.materials[&records[bond.b.as_str()].material];
        for material in [material_a, material_b] {
            validate_material(material)?;
        }
        let young = harmonic(
            material_a.young_modulus as f64,
            material_b.young_modulus as f64,
        );
        let shear = harmonic(shear_modulus(material_a), shear_modulus(material_b));
        let normal_length = norm3(bond.normal.map(f64::from));
        if !normal_length.is_finite()
            || (normal_length - 1.0).abs() > 1e-3
            || !bond.area.is_finite()
            || bond.area <= 0.0
        {
            return invalid(&format!("Invalid interface area or normal for {}", bond.id));
        }
        let normal = scale3(bond.normal.map(f64::from), 1.0 / normal_length);
        let normal_outer = outer3(normal, normal);
        let projection = subm3(identity3(), normal_outer);
        let raw_q = bond.area_covariance.map(|row| row.map(f64::from));
        let q = mulm3(mulm3(projection, raw_q), projection);
        let axis = if normal[0].abs() < 0.7 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let tangent_raw = cross3(normal, axis);
        let tangent = scale3(tangent_raw, 1.0 / norm3(tangent_raw));
        let bitangent = cross3(normal, tangent);
        let qt = dot3(tangent, mul3(q, tangent));
        let qb = dot3(bitangent, mul3(q, bitangent));
        let qtb = dot3(tangent, mul3(q, bitangent));
        let determinant = qt * qb - qtb * qtb;
        if !q.into_iter().flatten().all(f64::is_finite)
            || qt <= 0.0
            || qb <= 0.0
            || determinant <= 0.0
        {
            return invalid(&format!(
                "Degenerate interface second moment for {}",
                bond.id
            ));
        }
        let inverse_q = [
            [qb / determinant, -qtb / determinant],
            [-qtb / determinant, qt / determinant],
        ];
        let trace = qt + qb;
        let lambda_max = 0.5 * (trace + ((qt - qb).powi(2) + 4.0 * qtb * qtb).sqrt());
        let area = bond.area as f64;
        let max_radius = 2.0 * (lambda_max / area).sqrt();
        let translation = addm3(
            scale_m3(normal_outer, young * area / length),
            scale_m3(projection, shear * area / length),
        );
        let cross_normal = skew3(normal);
        let bending = mulm3(mulm3(cross_normal, q), transpose3(cross_normal));
        let rotation = addm3(
            scale_m3(bending, young / length),
            scale_m3(normal_outer, shear * trace / length),
        );
        let mut stiffness = [[0.0; 6]; 6];
        for row in 0..3 {
            for column in 0..3 {
                stiffness[row][column] = translation[row][column];
                stiffness[row + 3][column + 3] = rotation[row][column];
            }
        }
        let center = bond.center.map(f64::from);
        elements.push(Element {
            id: bond.id.clone(),
            a,
            b,
            center,
            normal,
            area,
            q,
            tangent,
            bitangent,
            inverse_q,
            max_radius,
            stiffness,
            ja: jacobian(sub3(center, nodes[a].center), -1.0, length_scale),
            jb: jacobian(sub3(center, nodes[b].center), 1.0, length_scale),
            tension_strength: f64::from(
                material_a.tension_strength.min(material_b.tension_strength),
            ),
            compression_strength: f64::from(
                material_a
                    .compression_strength
                    .min(material_b.compression_strength),
            ),
            shear_strength: f64::from(material_a.shear_strength.min(material_b.shear_strength)),
        });
    }
    let mut rows = vec![BTreeMap::new(); free_count];
    for element in &elements {
        for (i, ji) in [(element.a, element.ja), (element.b, element.jb)] {
            for (j, jj) in [(element.a, element.ja), (element.b, element.jb)] {
                if let (Some(i), Some(j)) = (nodes[i].free, nodes[j].free) {
                    let block = rows[i].entry(j).or_insert([[0.0; 6]; 6]);
                    add_m6(block, transpose_mul6(ji, mulm6(element.stiffness, jj)));
                }
            }
        }
    }
    let mut load = vec![0.0; free_count * 6];
    for node in &nodes {
        if let Some(free) = node.free {
            load[free * 6..free * 6 + 3].copy_from_slice(&node.load.force);
            for axis in 0..3 {
                load[free * 6 + 3 + axis] = node.load.moment[axis] / length_scale;
            }
        }
    }
    let mut row_offsets = Vec::with_capacity(rows.len() + 1);
    let mut blocks = Vec::with_capacity(rows.iter().map(BTreeMap::len).sum());
    let mut diagonal_blocks = Vec::with_capacity(rows.len());
    row_offsets.push(0);
    for (index, row) in rows.into_iter().enumerate() {
        let mut diagonal = None;
        for (column, values) in row {
            if index == column {
                diagonal = Some(blocks.len());
            }
            blocks.push(StiffnessBlock { column, values });
        }
        diagonal_blocks.push(diagonal);
        row_offsets.push(blocks.len());
    }
    Ok(Model {
        nodes,
        elements,
        row_offsets,
        blocks,
        diagonal_blocks,
        load,
        length_scale,
    })
}

impl Model {
    fn multiply(&self, x: &[f64], result: &mut [f64]) {
        for (i, offsets) in self.row_offsets.windows(2).enumerate() {
            let mut sum = [0.0; 6];
            for block in &self.blocks[offsets[0]..offsets[1]] {
                let column: &V6 = x[block.column * 6..block.column * 6 + 6]
                    .try_into()
                    .unwrap();
                for (value, row) in sum.iter_mut().zip(&block.values) {
                    *value += dot6(row, column);
                }
            }
            result[i * 6..i * 6 + 6].copy_from_slice(&sum);
        }
    }

    fn recover(
        &self,
        x: &Displacements,
        iterations: usize,
        relative_residual: f64,
        section_vertices: Option<&BTreeMap<String, Vec<V3>>>,
    ) -> StressSolution {
        let q = self
            .nodes
            .iter()
            .map(|node| {
                node.free.map_or([0.0; 6], |free| {
                    x[free * 6..free * 6 + 6].try_into().unwrap()
                })
            })
            .collect::<Vec<V6>>();
        let mut internal = vec![[0.0; 6]; self.nodes.len()];
        let mut bonds = BTreeMap::new();
        for element in &self.elements {
            let deformation = precise_deformation(self, element, &x.high, x.low.as_deref());
            let traction = mul6(element.stiffness, deformation);
            for (node, jacobian) in [(element.a, element.ja), (element.b, element.jb)] {
                let gradient = transpose_mul_vec6(jacobian, traction);
                for axis in 0..6 {
                    internal[node][axis] += gradient[axis];
                }
            }
            bonds.insert(
                element.id.clone(),
                element.stress(
                    traction,
                    section_vertices.and_then(|vertices| vertices.get(&element.id)),
                ),
            );
        }
        let reactions = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.fixed)
            .map(|(index, node)| {
                (
                    node.id.clone(),
                    NodeLoad {
                        force: std::array::from_fn(|axis| {
                            internal[index][axis] - node.load.force[axis]
                        }),
                        moment: std::array::from_fn(|axis| {
                            internal[index][axis + 3] * self.length_scale - node.load.moment[axis]
                        }),
                    },
                )
            })
            .collect();
        let displacements = self
            .nodes
            .iter()
            .zip(q)
            .map(|(node, mut q)| {
                for angle in &mut q[3..] {
                    *angle /= self.length_scale;
                }
                (node.id.clone(), q)
            })
            .collect();
        StressSolution {
            bonds,
            reactions,
            displacements,
            iterations,
            relative_residual,
        }
    }
}

impl Element {
    fn stress(&self, traction: V6, vertices: Option<&Vec<V3>>) -> BondStress {
        let force = [traction[0], traction[1], traction[2]];
        let moment = [traction[3], traction[4], traction[5]];
        let axial = dot3(force, self.normal) / self.area;
        let bending_vector = cross3(self.normal, moment);
        let rhs = [
            dot3(bending_vector, self.tangent),
            dot3(bending_vector, self.bitangent),
        ];
        let gradient2 = [
            self.inverse_q[0][0] * rhs[0] + self.inverse_q[0][1] * rhs[1],
            self.inverse_q[1][0] * rhs[0] + self.inverse_q[1][1] * rhs[1],
        ];
        let gradient = add3(
            scale3(self.tangent, gradient2[0]),
            scale3(self.bitangent, gradient2[1]),
        );
        let polar = dot3(self.tangent, mul3(self.q, self.tangent))
            + dot3(self.bitangent, mul3(self.q, self.bitangent));
        let twist = dot3(moment, self.normal) / polar;
        let direct_shear = scale3(
            sub3(force, scale3(self.normal, dot3(force, self.normal))),
            1.0 / self.area,
        );
        let (tension, compression, shear, section_exact) =
            if let Some(vertices) = vertices.filter(|vertices| !vertices.is_empty()) {
                let mut tension = 0.0_f64;
                let mut compression = 0.0_f64;
                let mut shear = 0.0_f64;
                for &point in vertices {
                    let r = sub3(point, self.center);
                    let normal_stress = axial + dot3(gradient, r);
                    tension = tension.max(normal_stress);
                    compression = compression.max(-normal_stress);
                    shear = shear.max(norm3(add3(
                        direct_shear,
                        scale3(cross3(self.normal, r), twist),
                    )));
                }
                (tension, compression, shear, true)
            } else {
                let bending = 2.0
                    * (dot3(gradient, mul3(self.q, gradient)) / self.area)
                        .max(0.0)
                        .sqrt();
                (
                    (axial + bending).max(0.0),
                    (-axial + bending).max(0.0),
                    norm3(direct_shear) + twist.abs() * self.max_radius,
                    false,
                )
            };
        BondStress {
            force,
            moment,
            tension,
            compression,
            shear,
            utilization: (tension / self.tension_strength)
                .max(compression / self.compression_strength)
                .max(shear / self.shear_strength),
            section_exact,
        }
    }
}

struct Displacements {
    high: Vec<f64>,
    // Keep small strain corrections when a weak support permits a large rigid motion.
    low: Option<Vec<f64>>,
}

impl std::ops::Deref for Displacements {
    type Target = [f64];
    fn deref(&self) -> &[f64] {
        &self.high
    }
}

#[derive(Clone, Copy, Default)]
struct Compensated {
    high: f64,
    low: f64,
}

impl Compensated {
    fn add(&mut self, value: f64) {
        let sum = self.high + value;
        let virtual_value = sum - self.high;
        let error = (self.high - (sum - virtual_value)) + (value - virtual_value);
        let tail = self.low + error;
        self.high = sum + tail;
        self.low = tail - (self.high - sum);
    }

    fn product(&mut self, a: f64, b: f64) {
        let product = a * b;
        self.add(product);
        self.add(a.mul_add(b, -product));
    }

    fn value(self) -> f64 {
        self.high + self.low
    }
}

fn precise_deformation(model: &Model, element: &Element, high: &[f64], low: Option<&[f64]>) -> V6 {
    std::array::from_fn(|row| {
        let mut sum = Compensated::default();
        for (node, jacobian) in [(element.a, &element.ja), (element.b, &element.jb)] {
            if let Some(index) = model.nodes[node].free {
                for (column, &coefficient) in jacobian[row].iter().enumerate() {
                    if coefficient == 0.0 {
                        continue;
                    }
                    let offset = index * 6 + column;
                    sum.product(coefficient, high[offset]);
                    if let Some(low) = low {
                        sum.product(coefficient, low[offset]);
                    }
                }
            }
        }
        sum.value()
    })
}

fn physical_residual(model: &Model, x: &Displacements, residual: &mut [f64]) {
    let mut internal = vec![Compensated::default(); residual.len()];
    for element in &model.elements {
        let deformation = precise_deformation(model, element, &x.high, x.low.as_deref());
        let traction = mul6(element.stiffness, deformation);
        for (node, jacobian) in [(element.a, element.ja), (element.b, element.jb)] {
            if let Some(index) = model.nodes[node].free {
                let force = transpose_mul_vec6(jacobian, traction);
                for axis in 0..6 {
                    internal[index * 6 + axis].add(force[axis]);
                }
            }
        }
    }
    for ((residual, &load), internal) in residual.iter_mut().zip(&model.load).zip(internal) {
        *residual = load - internal.value();
    }
}

struct SparseFactor {
    lower: Vec<BTreeMap<usize, M6>>,
    diagonal: Vec<M6>,
    upper: Vec<Vec<(usize, M6)>>,
    permutation: Vec<usize>,
}

impl SparseFactor {
    fn build(model: &Model, iteration: usize) -> Result<Self, SolveError> {
        let n = model.diagonal_blocks.len();
        let mut graph = vec![BTreeSet::new(); n];
        for (index, offsets) in model.row_offsets.windows(2).enumerate() {
            for block in &model.blocks[offsets[0]..offsets[1]] {
                if block.column != index {
                    graph[index].insert(block.column);
                }
            }
        }
        let mut remaining = BTreeSet::from_iter(0..n);
        let mut permutation = Vec::with_capacity(n);
        while !remaining.is_empty() {
            let chosen = *remaining
                .iter()
                .min_by_key(|&&index| (graph[index].len(), index))
                .unwrap();
            let neighbors = graph[chosen].iter().copied().collect::<Vec<_>>();
            for &i in &neighbors {
                graph[i].remove(&chosen);
                for &j in &neighbors {
                    if i != j {
                        graph[i].insert(j);
                    }
                }
            }
            remaining.remove(&chosen);
            permutation.push(chosen);
        }
        let mut inverse = vec![0; n];
        for (index, &old) in permutation.iter().enumerate() {
            inverse[old] = index;
        }
        let mut rows = vec![BTreeMap::new(); n];
        for (old, offsets) in model.row_offsets.windows(2).enumerate() {
            let i = inverse[old];
            for block in &model.blocks[offsets[0]..offsets[1]] {
                let j = inverse[block.column];
                if j <= i {
                    rows[i].insert(j, block.values);
                }
            }
        }
        let mut lower = vec![BTreeMap::new(); n];
        let mut diagonal = Vec::with_capacity(n);
        let mut upper = vec![Vec::new(); n];
        for k in 0..n {
            let value = rows[k].remove(&k).unwrap();
            let Some(chol) = cholesky6(value) else {
                return Err(SolveError::NumericalBreakdown { iteration });
            };
            diagonal.push(chol);
            let mut column = Vec::new();
            for (i, row) in rows.iter_mut().enumerate().skip(k + 1) {
                if let Some(value) = row.remove(&k) {
                    let factor = std::array::from_fn(|r| forward6(chol, value[r]));
                    lower[i].insert(k, factor);
                    upper[k].push((i, factor));
                    column.push((i, factor));
                }
            }
            for (a, &(i, li)) in column.iter().enumerate() {
                for &(j, lj) in &column[..=a] {
                    let product = mul_transpose6(li, lj);
                    let value = rows[i].entry(j).or_insert([[0.0; 6]; 6]);
                    for (row, product) in value.iter_mut().zip(product) {
                        for (value, product) in row.iter_mut().zip(product) {
                            *value -= product;
                        }
                    }
                }
            }
        }
        Ok(Self {
            lower,
            diagonal,
            upper,
            permutation,
        })
    }

    fn solve(&self, residual: &[f64], output: &mut [f64]) {
        for i in 0..self.diagonal.len() {
            let index = self.permutation[i];
            let mut rhs: V6 = residual[index * 6..index * 6 + 6].try_into().unwrap();
            for (&j, block) in &self.lower[i] {
                let j = self.permutation[j];
                let x = output[j * 6..j * 6 + 6].try_into().unwrap();
                let delta = mul6(*block, x);
                for axis in 0..6 {
                    rhs[axis] -= delta[axis];
                }
            }
            output[index * 6..index * 6 + 6].copy_from_slice(&forward6(self.diagonal[i], rhs));
        }
        for i in (0..self.diagonal.len()).rev() {
            let index = self.permutation[i];
            let mut rhs: V6 = output[index * 6..index * 6 + 6].try_into().unwrap();
            for (j, block) in &self.upper[i] {
                let j = self.permutation[*j];
                let x = output[j * 6..j * 6 + 6].try_into().unwrap();
                let delta = transpose_mul_vec6(*block, x);
                for axis in 0..6 {
                    rhs[axis] -= delta[axis];
                }
            }
            output[index * 6..index * 6 + 6].copy_from_slice(&backward6(self.diagonal[i], rhs));
        }
    }
}

fn mul_transpose6(a: M6, b: M6) -> M6 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..6).map(|k| a[i][k] * b[j][k]).sum()))
}

fn forward6(factor: M6, rhs: V6) -> V6 {
    let mut x = [0.0; 6];
    for i in 0..6 {
        x[i] = (rhs[i] - (0..i).map(|j| factor[i][j] * x[j]).sum::<f64>()) / factor[i][i];
    }
    x
}

fn backward6(factor: M6, rhs: V6) -> V6 {
    let mut x = [0.0; 6];
    for i in (0..6).rev() {
        x[i] = (rhs[i] - (i + 1..6).map(|j| factor[j][i] * x[j]).sum::<f64>()) / factor[i][i];
    }
    x
}

fn refine(
    model: &Model,
    settings: &SolverSettings,
    high: Vec<f64>,
    completed: usize,
    norm_b: f64,
) -> Result<(Displacements, usize, f64), SolveError> {
    let mut x = Displacements {
        low: Some(vec![0.0; high.len()]),
        high,
    };
    let mut residual = vec![0.0; x.len()];
    physical_residual(model, &x, &mut residual);
    let mut relative_residual = dot(&residual, &residual).sqrt() / norm_b;
    if relative_residual <= settings.relative_tolerance {
        return Ok((x, completed, relative_residual));
    }
    if completed >= settings.max_iterations {
        return Err(SolveError::NonConvergence {
            iterations: settings.max_iterations,
            relative_residual,
        });
    }
    let factor = SparseFactor::build(model, completed)?;
    let mut correction = vec![0.0; x.len()];
    for iteration in completed + 1..=settings.max_iterations {
        factor.solve(&residual, &mut correction);
        if correction.iter().any(|value| !value.is_finite()) {
            return Err(SolveError::NumericalBreakdown { iteration });
        }
        for ((high, low), correction) in x
            .high
            .iter_mut()
            .zip(x.low.as_mut().unwrap())
            .zip(&correction)
        {
            let mut value = Compensated {
                high: *high,
                low: *low,
            };
            value.add(*correction);
            *high = value.high;
            *low = value.low;
        }
        physical_residual(model, &x, &mut residual);
        relative_residual = dot(&residual, &residual).sqrt() / norm_b;
        if relative_residual <= settings.relative_tolerance {
            return Ok((x, iteration, relative_residual));
        }
        if !relative_residual.is_finite() {
            return Err(SolveError::NumericalBreakdown { iteration });
        }
    }
    Err(SolveError::NonConvergence {
        iterations: settings.max_iterations,
        relative_residual,
    })
}

fn pcg(
    model: &Model,
    settings: &SolverSettings,
) -> Result<(Displacements, usize, f64), SolveError> {
    if !settings.relative_tolerance.is_finite()
        || settings.relative_tolerance <= 0.0
        || settings.relative_tolerance >= 1.0
    {
        return invalid("Relative solver tolerance must be between zero and one");
    }
    let mut factors = Vec::new();
    for diagonal in &model.diagonal_blocks {
        let Some(diagonal) = diagonal else {
            return invalid("Missing stiffness diagonal");
        };
        let Some(factor) = cholesky6(model.blocks[*diagonal].values) else {
            return invalid("Non-positive local stiffness block");
        };
        factors.push(factor);
    }
    let mut x = vec![0.0; model.load.len()];
    let norm_b = dot(&model.load, &model.load).sqrt();
    if norm_b == 0.0 {
        return Ok((Displacements { high: x, low: None }, 0, 0.0));
    }
    let mut r = model.load.clone();
    let mut z = vec![0.0; r.len()];
    precondition(&factors, &r, &mut z);
    let mut p = z.clone();
    let mut ap = vec![0.0; r.len()];
    let mut ax = vec![0.0; r.len()];
    let mut rz = dot(&r, &z);
    for iteration in 1..=settings.max_iterations {
        if iteration == 513 {
            // Difficult graphs use the remaining budget for physical-residual refinement.
            return refine(model, settings, x, iteration - 1, norm_b);
        }
        model.multiply(&p, &mut ap);
        let curvature = dot(&p, &ap);
        if !curvature.is_finite() || curvature <= 0.0 || !rz.is_finite() || rz <= 0.0 {
            return Err(SolveError::NumericalBreakdown { iteration });
        }
        let alpha = rz / curvature;
        for i in 0..x.len() {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        let recursive_residual = dot(&r, &r).sqrt() / norm_b;
        let mut restart = false;
        if recursive_residual <= settings.relative_tolerance || iteration % 128 == 0 {
            model.multiply(&x, &mut ax);
            for (ax, b) in ax.iter_mut().zip(&model.load) {
                *ax = b - *ax;
            }
            let true_residual = dot(&ax, &ax).sqrt() / norm_b;
            if true_residual <= settings.relative_tolerance {
                let solution = Displacements { high: x, low: None };
                physical_residual(model, &solution, &mut ax);
                let physical = dot(&ax, &ax).sqrt() / norm_b;
                if physical <= settings.relative_tolerance {
                    return Ok((solution, iteration, physical));
                }
                return refine(model, settings, solution.high, iteration, norm_b);
            }
            if recursive_residual <= settings.relative_tolerance {
                r.copy_from_slice(&ax);
                // Replacing the residual invalidates the old conjugate direction.
                restart = true;
            }
        }
        precondition(&factors, &r, &mut z);
        let next_rz = dot(&r, &z);
        let beta = if restart { 0.0 } else { next_rz / rz };
        for i in 0..p.len() {
            p[i] = z[i] + beta * p[i];
        }
        rz = next_rz;
    }
    let solution = Displacements { high: x, low: None };
    physical_residual(model, &solution, &mut ax);
    let residual = dot(&ax, &ax).sqrt() / norm_b;
    if residual <= settings.relative_tolerance {
        return Ok((solution, settings.max_iterations, residual));
    }
    Err(SolveError::NonConvergence {
        iterations: settings.max_iterations,
        relative_residual: residual,
    })
}

fn precondition(factors: &[M6], residual: &[f64], output: &mut [f64]) {
    for ((factor, rhs), result) in factors
        .iter()
        .zip(residual.as_chunks::<6>().0)
        .zip(output.as_chunks_mut::<6>().0)
    {
        let mut x = [0.0; 6];
        for i in 0..6 {
            x[i] = (rhs[i] - (0..i).map(|j| factor[i][j] * x[j]).sum::<f64>()) / factor[i][i];
        }
        for i in (0..6).rev() {
            x[i] = (x[i] - (i + 1..6).map(|j| factor[j][i] * x[j]).sum::<f64>()) / factor[i][i];
        }
        *result = x;
    }
}

fn cholesky6(matrix: M6) -> Option<M6> {
    let mut factor = [[0.0; 6]; 6];
    for i in 0..6 {
        for j in 0..=i {
            let value = matrix[i][j] - (0..j).map(|k| factor[i][k] * factor[j][k]).sum::<f64>();
            if i == j {
                if !value.is_finite() || value <= 0.0 {
                    return None;
                }
                factor[i][j] = value.sqrt();
            } else {
                factor[i][j] = value / factor[j][j];
            }
        }
    }
    Some(factor)
}

fn validate_material(material: &PhysicalMaterial) -> Result<(), SolveError> {
    if [
        material.young_modulus,
        material.tension_strength,
        material.compression_strength,
        material.shear_strength,
    ]
    .into_iter()
    .any(|value| !value.is_finite() || value <= 0.0)
        || !material.poisson_ratio.is_finite()
        || material.poisson_ratio <= -1.0
        || material.poisson_ratio >= 0.5
    {
        return invalid("Invalid elastic material or strength");
    }
    Ok(())
}

fn shear_modulus(material: &PhysicalMaterial) -> f64 {
    f64::from(material.young_modulus) / (2.0 * (1.0 + f64::from(material.poisson_ratio)))
}
fn harmonic(a: f64, b: f64) -> f64 {
    2.0 / (1.0 / a + 1.0 / b)
}
fn invalid<T>(message: &str) -> Result<T, SolveError> {
    Err(SolveError::InvalidModel(message.into()))
}
fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
fn dot6(a: &V6, b: &V6) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3] + a[4] * b[4] + a[5] * b[5]
}
fn add3(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] + b[i])
}
fn sub3(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
fn scale3(a: V3, k: f64) -> V3 {
    a.map(|a| a * k)
}
fn dot3(a: V3, b: V3) -> f64 {
    dot(&a, &b)
}
fn norm3(a: V3) -> f64 {
    dot3(a, a).sqrt()
}
fn cross3(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn identity3() -> M3 {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}
fn outer3(a: V3, b: V3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| a[i] * b[j]))
}
fn skew3([x, y, z]: V3) -> M3 {
    [[0.0, -z, y], [z, 0.0, -x], [-y, x, 0.0]]
}
fn mul3(a: M3, x: V3) -> V3 {
    a.map(|row| dot3(row, x))
}
fn addm3(a: M3, b: M3) -> M3 {
    std::array::from_fn(|i| add3(a[i], b[i]))
}
fn subm3(a: M3, b: M3) -> M3 {
    std::array::from_fn(|i| sub3(a[i], b[i]))
}
fn scale_m3(a: M3, k: f64) -> M3 {
    a.map(|row| scale3(row, k))
}
fn transpose3(a: M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| a[j][i]))
}
fn mulm3(a: M3, b: M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}
#[cfg(test)]
fn add6(a: V6, b: V6) -> V6 {
    std::array::from_fn(|i| a[i] + b[i])
}
fn mul6(a: M6, b: V6) -> V6 {
    a.map(|row| dot(&row, &b))
}
fn transpose_mul_vec6(a: M6, b: V6) -> V6 {
    std::array::from_fn(|i| (0..6).map(|j| a[j][i] * b[j]).sum())
}
fn mulm6(a: M6, b: M6) -> M6 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..6).map(|k| a[i][k] * b[k][j]).sum()))
}
fn transpose_mul6(a: M6, b: M6) -> M6 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..6).map(|k| a[k][i] * b[k][j]).sum()))
}
fn add_m6(a: &mut M6, b: M6) {
    for i in 0..6 {
        for j in 0..6 {
            a[i][j] += b[i][j];
        }
    }
}

fn jacobian(offset: V3, sign: f64, length_scale: f64) -> M6 {
    let mut matrix = [[0.0; 6]; 6];
    let skew = skew3(offset);
    for i in 0..3 {
        matrix[i][i] = sign;
        matrix[i + 3][i + 3] = sign / length_scale;
        for j in 0..3 {
            matrix[i][j + 3] = -sign * skew[i][j] / length_scale;
        }
    }
    matrix
}

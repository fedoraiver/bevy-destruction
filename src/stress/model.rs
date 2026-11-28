use std::collections::{BTreeMap, BTreeSet};

use crate::asset::{Manifest, PhysicalMaterial};

use super::{
    BondStress, NodeLoad, SolveError, StressSolution, invalid,
    math::{
        Compensated, M3, M6, V3, V6, add_m6, add3, addm3, cross3, dot3, dot6, identity3, jacobian,
        mul3, mul6, mulm3, mulm6, norm3, outer3, scale_m3, scale3, skew3, sub3, subm3,
        transpose_mul_vec6, transpose_mul6, transpose3,
    },
    node_center,
    solver::Displacements,
};

pub(super) struct Node {
    pub(super) id: String,
    pub(super) center: V3,
    pub(super) load: NodeLoad,
    pub(super) fixed: bool,
    free: Option<usize>,
}

pub(super) struct Element {
    pub(super) id: String,
    pub(super) a: usize,
    pub(super) b: usize,
    pub(super) center: V3,
    pub(super) bond_center: V3,
    vertices: Vec<V3>,
    normal: V3,
    pub(super) area: f64,
    q: M3,
    tangent: V3,
    bitangent: V3,
    inverse_q: [[f64; 2]; 2],
    stiffness: M6,
    pub(super) ja: M6,
    pub(super) jb: M6,
    tension_strength: f64,
    compression_strength: f64,
    shear_strength: f64,
}

pub(super) struct Model {
    pub(super) nodes: Vec<Node>,
    pub(super) elements: Vec<Element>,
    pub(super) row_offsets: Vec<usize>,
    pub(super) blocks: Vec<StiffnessBlock>,
    pub(super) diagonal_blocks: Vec<Option<usize>>,
    pub(super) load: Vec<f64>,
    pub(super) length_scale: f64,
}

pub(super) struct StiffnessBlock {
    pub(super) column: usize,
    pub(super) values: M6,
}

pub(super) fn assemble_with_gauge(
    manifest: &Manifest,
    chunks: &[String],
    active_bonds: &BTreeMap<String, bool>,
    loads: &BTreeMap<String, NodeLoad>,
    gauge: Option<&str>,
) -> Result<Model, SolveError> {
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
        let center = node_center(record).to_array();
        let fixed = anchors.contains(id.as_str());
        let load = loads.get(id).copied().unwrap_or_default();
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
        // A shared equivalent length makes a planar interface invariant under subdivision.
        if bond.patches.is_empty() {
            return invalid(&format!("Missing v2 interface patches for {}", bond.id));
        }
        for patch in &bond.patches {
            let area = patch.area;
            let center = patch.center;
            let normal = patch.normal;
            let area_covariance = patch.area_covariance;
            if patch.vertices.len() < 3
                || !patch.vertices.iter().flatten().all(|v| v.is_finite())
                || !center.into_iter().all(f32::is_finite)
            {
                return invalid(&format!("Invalid v2 interface polygon for {}", bond.id));
            }
            let vertices = patch.vertices.iter().map(|p| p.map(f64::from)).collect();
            let normal_length = norm3(normal.map(f64::from));
            if !normal_length.is_finite()
                || (normal_length - 1.0).abs() > 1e-3
                || !area.is_finite()
                || area <= 0.0
            {
                return invalid(&format!("Invalid interface area or normal for {}", bond.id));
            }
            let normal = scale3(normal.map(f64::from), 1.0 / normal_length);
            let normal_outer = outer3(normal, normal);
            let projection = subm3(identity3(), normal_outer);
            let raw_q = area_covariance.map(|row| row.map(f64::from));
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
            let area = area as f64;
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
            let center = center.map(f64::from);
            elements.push(Element {
                id: bond.id.clone(),
                a,
                b,
                center,
                bond_center: bond.center.map(f64::from),
                vertices,
                normal,
                area,
                q,
                tangent,
                bitangent,
                inverse_q,
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
    pub(super) fn multiply(&self, x: &[f64], result: &mut [f64]) {
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

    pub(super) fn recover(
        &self,
        x: &Displacements,
        iterations: usize,
        relative_residual: f64,
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
            let mut stress = element.stress(traction, 0.5 * dot6(&deformation, &traction));
            stress.moment = add3(
                stress.moment,
                cross3(sub3(element.center, element.bond_center), stress.force),
            );
            bonds
                .entry(element.id.clone())
                .and_modify(|total: &mut BondStress| {
                    total.force = add3(total.force, stress.force);
                    total.moment = add3(total.moment, stress.moment);
                    total.tension = total.tension.max(stress.tension);
                    total.compression = total.compression.max(stress.compression);
                    total.shear = total.shear.max(stress.shear);
                    total.utilization = total.utilization.max(stress.utilization);
                    total.strain_energy += stress.strain_energy;
                    total.section_exact &= stress.section_exact;
                })
                .or_insert(stress);
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
    fn stress(&self, traction: V6, strain_energy: f64) -> BondStress {
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
        let mut tension = 0.0_f64;
        let mut compression = 0.0_f64;
        let mut shear = 0.0_f64;
        for &point in &self.vertices {
            let r = sub3(point, self.center);
            let normal_stress = axial + dot3(gradient, r);
            tension = tension.max(normal_stress);
            compression = compression.max(-normal_stress);
            shear = shear.max(norm3(add3(
                direct_shear,
                scale3(cross3(self.normal, r), twist),
            )));
        }
        BondStress {
            force,
            moment,
            tension,
            compression,
            shear,
            utilization: (tension / self.tension_strength)
                .max(compression / self.compression_strength)
                .max(shear / self.shear_strength),
            strain_energy,
            section_exact: true,
        }
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

pub(super) fn physical_residual(model: &Model, x: &Displacements, residual: &mut [f64]) {
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

pub(super) fn shear_modulus(material: &PhysicalMaterial) -> f64 {
    f64::from(material.young_modulus) / (2.0 * (1.0 + f64::from(material.poisson_ratio)))
}
fn harmonic(a: f64, b: f64) -> f64 {
    2.0 / (1.0 / a + 1.0 / b)
}

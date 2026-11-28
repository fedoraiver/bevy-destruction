mod math;
mod model;
mod solver;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use bevy::{
    math::DVec3,
    prelude::{Reflect, Vec3},
};

use crate::asset::{ChunkRecord, Manifest};

use self::{
    math::{V3, V6, add3, cross3, norm3, scale3, sub3},
    model::assemble_with_gauge,
    solver::pcg,
};

#[cfg(test)]
use self::{
    math::{addm3, dot, dot3, dot6, identity3, mul6, outer3, scale_m3, subm3},
    model::{Model, shear_modulus},
    solver::refine,
};
#[cfg(test)]
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
#[cfg_attr(feature = "brp", derive(Reflect))]
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
#[cfg_attr(feature = "brp", derive(Reflect))]
pub struct NodeLoad {
    pub force: V3,
    pub moment: V3,
}

impl NodeLoad {
    /// Shifts the moment using the vector from the new reference to the old one.
    pub(crate) fn at_reference(self, offset: DVec3) -> Self {
        Self {
            moment: (DVec3::from_array(self.moment) + offset.cross(DVec3::from_array(self.force)))
                .to_array(),
            ..self
        }
    }
}

pub(crate) fn node_center(record: &ChunkRecord) -> DVec3 {
    record
        .transform()
        .transform_point(Vec3::from(record.center_of_mass))
        .as_dvec3()
}

#[derive(Debug, Clone, Reflect)]
pub struct BondStress {
    /// Force and couple acting on endpoint a at the interface center.
    pub force: V3,
    pub moment: V3,
    pub tension: f64,
    pub compression: f64,
    pub shear: f64,
    pub utilization: f64,
    /// 各弹性面片储存的应变能之和，单位 J。
    pub strain_energy: f64,
    pub section_exact: bool,
}

#[derive(Debug, Clone)]
pub struct StressSolution {
    pub bonds: BTreeMap<String, BondStress>,
    /// Support actions at the authored structural nodes.
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

pub fn solve(
    manifest: &Manifest,
    chunks: &[String],
    active_bonds: &BTreeMap<String, bool>,
    loads: &BTreeMap<String, NodeLoad>,
    settings: &SolverSettings,
) -> Result<StressSolution, SolveError> {
    let model = assemble_with_gauge(manifest, chunks, active_bonds, loads, None)?;
    let (x, iterations, relative_residual) = pcg(&model, settings)?;
    Ok(model.recover(&x, iterations, relative_residual))
}

/// Solves balanced free-body loads; the gauge removes rigid modes, not a physical support.
/// Asset anchors are ignored and remain unchanged.
pub fn solve_with_gauge(
    manifest: &Manifest,
    chunks: &[String],
    active_bonds: &BTreeMap<String, bool>,
    external_loads: &BTreeMap<String, NodeLoad>,
    settings: &SolverSettings,
    gauge: &str,
) -> Result<StressSolution, SolveError> {
    let model = assemble_with_gauge(manifest, chunks, active_bonds, external_loads, Some(gauge))?;
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
    Ok(model.recover(&x, iterations, relative_residual))
}

fn invalid<T>(message: &str) -> Result<T, SolveError> {
    Err(SolveError::InvalidModel(message.into()))
}

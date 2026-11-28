use std::collections::{BTreeMap, BTreeSet};

use super::{
    SolveError, SolverSettings, invalid,
    math::{Compensated, M6, V6, dot, mul6, transpose_mul_vec6},
    model::{Model, physical_residual},
};

pub(super) struct Displacements {
    pub(super) high: Vec<f64>,
    // Keep small strain corrections when a weak support permits a large rigid motion.
    pub(super) low: Option<Vec<f64>>,
}

impl std::ops::Deref for Displacements {
    type Target = [f64];
    fn deref(&self) -> &[f64] {
        &self.high
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

pub(super) fn refine(
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

pub(super) fn pcg(
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

//! Shared contact geometry and fracture rim interfaces.

mod contact;
mod polygons;
mod rims;

pub use contact::build_patches;
pub use rims::detail_mesh;

use bevy::prelude::Vec3;

#[cfg(test)]
pub(crate) use contact::contact_surface_mesh;
#[cfg(test)]
use {
    crate::asset::Manifest,
    bevy::{
        math::{DVec2, DVec3},
        prelude::Mesh,
    },
    contact::{Plane, mesh_patch_coverage},
    polygons::{connected_polygons, convex_hull, polygon_area},
    std::collections::BTreeMap,
};

#[cfg(test)]
mod tests;

const CONTACT_TOLERANCE: f64 = 0.00004;
pub const MIN_VISUAL_CONTACT_AREA: f32 = 1e-7;

#[derive(Clone, Default)]
pub struct SurfaceGeometry {
    pub triangles: Vec<[Vec3; 3]>,
    pub exterior: Vec<[Vec3; 3]>,
}

#[derive(Clone, Debug)]
pub struct RimEdge {
    pub points: [Vec3; 2],
    pub exterior_a: Option<Vec3>,
    pub exterior_b: Option<Vec3>,
    pub thickness_a: f32,
    pub thickness_b: f32,
}

#[derive(Clone, Debug)]
pub struct ContactPatch {
    pub id: String,
    pub bond_id: String,
    pub a: String,
    pub b: String,
    pub normal: Vec3,
    pub triangles: Vec<[Vec3; 3]>,
    pub area: f32,
    pub rims: Vec<RimEdge>,
}

impl ContactPatch {
    pub fn exposed_survivor(&self, alive: impl Fn(&str) -> bool) -> Option<&str> {
        match (alive(&self.a), alive(&self.b)) {
            (true, false) => Some(&self.a),
            (false, true) => Some(&self.b),
            _ => None,
        }
    }
}

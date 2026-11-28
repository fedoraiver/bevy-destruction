mod damage;
mod feedback;
mod input;
mod selection;

use bevy::prelude::*;

pub(crate) use damage::apply_damage;
pub(crate) use feedback::build_bond_feedback;
pub(crate) use input::cursor_damage;

#[cfg(test)]
use crate::scene::{BuildingInstance, ChunkInstance, PatchInstance};
#[cfg(test)]
use avian3d::prelude::{ColliderOf, MassProperties3d, Position, RigidBody, Rotation};
#[cfg(test)]
use selection::{closest_on_triangle, surface_distance};

pub const BREAK_RADIUS: f32 = 0.4;
pub const BREAK_LIMIT: usize = 6;

pub struct InteractionPlugin;

impl Plugin for InteractionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DamageRequest>()
            .add_systems(Update, (cursor_damage, apply_damage).chain());
    }
}

#[derive(Message)]
#[cfg_attr(feature = "brp", derive(Reflect), reflect(Message))]
pub(crate) struct DamageRequest {
    pub ray: Ray3d,
}

#[cfg(test)]
mod tests;

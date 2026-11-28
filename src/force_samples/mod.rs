use std::{
    collections::{BTreeMap, VecDeque},
    hash::{Hash, Hasher},
};

use bevy::prelude::*;

use crate::stress::BondStress;

pub const FORCE_SAMPLE_SECONDS: f64 = 0.7;
pub const IMPULSE_SAMPLE_SECONDS: f64 = 1.0;
const MAX_FORCE_SAMPLES: usize = 8;
pub const MAX_IMPULSE_SAMPLES: usize = 128;

#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
pub struct ForceCapture {
    pub enabled: bool,
}

#[derive(Reflect)]
pub struct DynamicForceSample {
    pub building: Entity,
    pub body: Entity,
    pub chunks: Vec<String>,
    pub bonds: BTreeMap<String, BondStress>,
    pub bond_signature: u64,
    pub captured_at: f64,
}

#[derive(Reflect)]
pub struct ImpulseSample {
    pub building: Entity,
    pub chunk: String,
    /// Actual normal impulse at one contact, in world space.
    pub impulse: Vec3,
    pub captured_at: f64,
}

#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
pub struct ForceSamples {
    pub dynamic: VecDeque<DynamicForceSample>,
    pub impulses: VecDeque<ImpulseSample>,
}

pub fn bond_signature<'a>(bonds: impl IntoIterator<Item = (&'a str, bool)>) -> u64 {
    let mut hash = std::hash::DefaultHasher::new();
    for (id, active) in bonds {
        id.hash(&mut hash);
        active.hash(&mut hash);
    }
    hash.finish()
}

impl ForceSamples {
    pub fn record_force(&mut self, sample: DynamicForceSample) {
        self.dynamic
            .retain(|old| (old.building, old.body) != (sample.building, sample.body));
        self.dynamic.push_back(sample);
        while self.dynamic.len() > MAX_FORCE_SAMPLES {
            self.dynamic.pop_front();
        }
    }

    pub fn record_impulse(&mut self, sample: ImpulseSample) {
        self.impulses.push_back(sample);
        while self.impulses.len() > MAX_IMPULSE_SAMPLES {
            self.impulses.pop_front();
        }
    }

    pub fn prune(
        &mut self,
        now: f64,
        mut force_current: impl FnMut(&DynamicForceSample) -> bool,
        mut impulse_current: impl FnMut(&ImpulseSample) -> bool,
    ) {
        self.dynamic.retain(|sample| {
            (0.0..=FORCE_SAMPLE_SECONDS).contains(&(now - sample.captured_at))
                && force_current(sample)
        });
        self.impulses.retain(|sample| {
            (0.0..=IMPULSE_SAMPLE_SECONDS).contains(&(now - sample.captured_at))
                && impulse_current(sample)
        });
    }

    pub fn clear(&mut self) {
        self.dynamic.clear();
        self.impulses.clear();
    }
}

#[cfg(test)]
mod tests;

use std::{path::PathBuf, str::FromStr, time::Duration};

use avian3d::prelude::Collider;
use bevy::prelude::*;

pub use crate::scene::DemoCamera;

use crate::{
    DemoOptions,
    camera::FreeCamera,
    diagnostics,
    projectiles::{ImpactStats, RigidProjectile},
    scene::BuildingInstance,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    Smoke,
    Physics,
    Stress,
    Material,
    Contact,
    Force,
    Landing,
    Collapse,
}

impl FromStr for Probe {
    type Err = String;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "smoke" => Ok(Self::Smoke),
            "physics" => Ok(Self::Physics),
            "stress" => Ok(Self::Stress),
            "material" => Ok(Self::Material),
            "contact" => Ok(Self::Contact),
            "force" => Ok(Self::Force),
            "landing" => Ok(Self::Landing),
            "collapse" => Ok(Self::Collapse),
            _ => Err(format!("Unknown acceptance probe: {name}")),
        }
    }
}

pub struct ProbeRun {
    pub probe: Probe,
    pub output: PathBuf,
    pub capture_replay: bool,
    pub profile: bool,
}

impl ProbeRun {
    pub fn configure(self, options: &mut DemoOptions, smoke_target: Handle<Image>) -> Probe {
        options.smoke_target = Some(smoke_target);
        options.capture_replay = self.capture_replay;
        options.profile = self.profile;
        let output = Some(self.output);
        match self.probe {
            Probe::Smoke => options.smoke_output = output,
            Probe::Physics => options.physics_smoke_output = output,
            Probe::Stress => options.stress_smoke_output = output,
            Probe::Material => options.material_smoke_output = output,
            Probe::Contact => options.contact_smoke_output = output,
            Probe::Force => options.force_smoke_output = output,
            Probe::Landing => options.landing_smoke_output = output,
            Probe::Collapse => options.collapse_smoke_output = output,
        }
        self.probe
    }
}

pub struct AcceptanceScenarioPlugin {
    pub probe: Option<Probe>,
}

impl Plugin for AcceptanceScenarioPlugin {
    fn build(&self, app: &mut App) {
        let Some(probe) = self.probe else {
            app.init_resource::<GameplayState>()
                .add_systems(Last, observe_gameplay);
            return;
        };

        let profile = app.world().resource::<DemoOptions>().profile;
        app.add_plugins(diagnostics::MetricsPlugin).insert_resource(
            bevy::time::TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(1.0 / 60.0)),
        );
        if profile {
            app.init_resource::<bevy::diagnostic::DiagnosticsStore>()
                .add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
        }
        match probe {
            Probe::Smoke => app.add_plugins(crate::smoke::SmokePlugin),
            Probe::Physics => app.add_plugins(crate::physics_smoke::PhysicsSmokePlugin),
            Probe::Stress => app.add_plugins(crate::stress_smoke::StressSmokePlugin),
            Probe::Material => app.add_plugins(crate::material_smoke::MaterialSmokePlugin),
            Probe::Contact => app.add_plugins(crate::contact_smoke::ContactSmokePlugin),
            Probe::Force => app.add_plugins(crate::force_smoke::ForceSmokePlugin),
            Probe::Landing => app.add_plugins(crate::landing_smoke::LandingSmokePlugin),
            Probe::Collapse => app.add_plugins(crate::collapse_smoke::CollapseSmokePlugin),
        };
    }
}

#[derive(Default, Reflect, Resource)]
#[reflect(Resource)]
struct GameplayState {
    frame: u64,
    asset: String,
    building: Option<Entity>,
    camera: Option<Entity>,
    ready: bool,
    load_error: Option<String>,
    camera_initialized: bool,
    translation: Vec3,
    rotation: Quat,
    removed_chunks: usize,
    live_bonds: usize,
    launched: u64,
    contacts: u64,
    sphere_contacts: u64,
    cube_contacts: u64,
    broken_bonds: u64,
    spheres: usize,
    cubes: usize,
}

fn observe_gameplay(
    options: Res<DemoOptions>,
    buildings: Query<(Entity, &BuildingInstance)>,
    cameras: Query<(Entity, &GlobalTransform, &FreeCamera), With<DemoCamera>>,
    projectiles: Query<&Collider, With<RigidProjectile>>,
    impacts: Res<ImpactStats>,
    mut state: ResMut<GameplayState>,
) {
    state.frame += 1;
    state.asset.clone_from(&options.asset);
    let building = buildings.single().ok();
    state.building = building.map(|(entity, _)| entity);
    state.load_error = building.and_then(|(_, instance)| instance.error.clone());
    state.ready = building.is_some_and(|(_, instance)| {
        instance.bounds.is_some() && !instance.clusters.is_empty() && instance.error.is_none()
    });
    state.removed_chunks = building.map_or(0, |(_, instance)| {
        instance
            .chunks
            .values()
            .filter(|chunk| !chunk.alive)
            .count()
    });
    state.live_bonds = building.map_or(0, |(_, instance)| {
        instance.bonds.values().filter(|&&alive| alive).count()
    });

    if let Ok((entity, transform, camera)) = cameras.single() {
        state.camera = Some(entity);
        state.camera_initialized = camera.is_initialized();
        state.translation = transform.translation();
        state.rotation = transform.rotation();
    } else {
        state.camera = None;
        state.camera_initialized = false;
    }

    state.launched = impacts.launched;
    state.contacts = impacts.contacts;
    state.sphere_contacts = impacts.sphere_contacts;
    state.cube_contacts = impacts.cube_contacts;
    state.broken_bonds = impacts.broken_bonds;
    state.spheres = 0;
    state.cubes = 0;
    for collider in &projectiles {
        let shape = collider.shape();
        state.spheres += usize::from(shape.as_ball().is_some());
        state.cubes += usize::from(shape.as_cuboid().is_some());
    }
}

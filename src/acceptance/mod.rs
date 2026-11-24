use std::{env, path::PathBuf, time::Duration};

use avian3d::prelude::Collider;
use bevy::prelude::*;
use serde::Deserialize;

pub use crate::scene::DemoCamera;

use crate::{
    DemoOptions, DestructionPlugin,
    camera::FreeCamera,
    diagnostics,
    projectiles::{ImpactStats, RigidProjectile},
    scene::{BondInstance, BuildingInstance, ChunkInstance},
};

const SCENARIO_ENV: &str = "BEVY_DESTRUCTION_ACCEPTANCE_SCENARIO";

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Probe {
    Smoke,
    Physics,
    Stress,
    Material,
    Contact,
    Force,
    Landing,
    Collapse,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProbeRun {
    asset: String,
    probe: Probe,
    output: PathBuf,
    #[serde(default)]
    capture_replay: bool,
    #[serde(default)]
    profile: bool,
    #[serde(default)]
    no_stress: bool,
    #[serde(default)]
    no_vfx: bool,
    #[serde(default)]
    no_detail: bool,
    #[serde(default)]
    no_soft_particles: bool,
}

pub struct AcceptanceScenario {
    options: DemoOptions,
    probe: Option<Probe>,
}

impl AcceptanceScenario {
    pub fn from_env(smoke_target: Handle<Image>) -> Result<Self, String> {
        let value = match env::var(SCENARIO_ENV) {
            Ok(value) => value,
            Err(env::VarError::NotPresent) => {
                return Ok(Self {
                    options: DemoOptions::default(),
                    probe: None,
                });
            }
            Err(error) => return Err(format!("{SCENARIO_ENV}: {error}")),
        };
        let run: ProbeRun =
            serde_json::from_str(&value).map_err(|error| format!("{SCENARIO_ENV}: {error}"))?;
        if (run.capture_replay || run.profile) && run.probe != Probe::Smoke {
            return Err("Replay capture and profiling require the smoke probe".into());
        }
        if run.capture_replay && run.profile {
            return Err("Replay capture and profiling are mutually exclusive".into());
        }

        let mut options = DemoOptions {
            asset: run.asset,
            no_stress: run.no_stress,
            no_vfx: run.no_vfx,
            no_detail: run.no_detail,
            no_soft_particles: run.no_soft_particles,
            capture_replay: run.capture_replay,
            smoke_target: Some(smoke_target),
            profile: run.profile,
            ..default()
        };
        let output = Some(run.output);
        match run.probe {
            Probe::Smoke => options.smoke_output = output,
            Probe::Physics => options.physics_smoke_output = output,
            Probe::Stress => options.stress_smoke_output = output,
            Probe::Material => options.material_smoke_output = output,
            Probe::Contact => options.contact_smoke_output = output,
            Probe::Force => options.force_smoke_output = output,
            Probe::Landing => options.landing_smoke_output = output,
            Probe::Collapse => options.collapse_smoke_output = output,
        }
        Ok(Self {
            options,
            probe: Some(run.probe),
        })
    }

    pub fn profile(&self) -> bool {
        self.options.profile
    }

    pub fn synchronous_pipelines(&self) -> bool {
        self.probe.is_some() && !self.profile()
    }

    pub fn install(self, app: &mut App) {
        let profile = self.profile();
        app.insert_resource(self.options)
            .add_plugins(DestructionPlugin);
        let Some(probe) = self.probe else {
            app.init_resource::<GameplayState>()
                .add_systems(Last, observe_gameplay);
            return;
        };

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
    graph: (Query<&ChunkInstance>, Query<&BondInstance>),
    cameras: Query<(Entity, &GlobalTransform, &FreeCamera), With<DemoCamera>>,
    projectiles: Query<&Collider, With<RigidProjectile>>,
    impacts: Res<ImpactStats>,
    mut state: ResMut<GameplayState>,
) {
    let (chunks, bonds) = graph;
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
            .filter_map(|&entity| chunks.get(entity).ok())
            .filter(|chunk| !chunk.alive)
            .count()
    });
    state.live_bonds = building.map_or(0, |(_, instance)| {
        instance
            .bonds
            .values()
            .filter(|&&entity| bonds.get(entity).is_ok_and(|bond| bond.active))
            .count()
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

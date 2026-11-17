use avian3d::prelude::*;
use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};

use crate::{
    DemoOptions,
    asset::DestructibleAsset,
    debug::FractureEdges,
    diagnostics::RunMetrics,
    physics::Cluster,
    projectiles::ImpactStats,
    scene::{BuildingInstance, DemoCamera, RemovedChunk},
};

pub struct LandingSmokePlugin;

impl Plugin for LandingSmokePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LandingState>().add_systems(
            Update,
            drive
                .after(crate::camera::move_camera)
                .after(crate::scene::instantiate_pending),
        );
    }
}

#[derive(Resource, Default)]
struct LandingState {
    waiting: u32,
    warmup: u32,
    frame: u32,
    upper: Vec<String>,
    cut: f32,
    initial_mass: f64,
    initial_bonds: usize,
    first_impact: Option<u32>,
    before_impact_broken: u64,
    largest_final: usize,
    internal_broken: usize,
    finite: bool,
    errors: Vec<String>,
}

fn capture(commands: &mut Commands, options: &DemoOptions, name: &str) {
    if !options.profile {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(
                options.landing_smoke_output.as_ref().unwrap().join(name),
            ));
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn drive(
    mut commands: Commands,
    options: Res<DemoOptions>,
    assets: Res<Assets<DestructibleAsset>>,
    mut state: ResMut<LandingState>,
    mut instances: Query<(Entity, &mut BuildingInstance)>,
    bodies: Query<(
        &Cluster,
        &Position,
        &Rotation,
        &LinearVelocity,
        &AngularVelocity,
        &Mass,
    )>,
    impacts: Res<ImpactStats>,
    mut cameras: Query<&mut Transform, With<DemoCamera>>,
    mut metrics: ResMut<RunMetrics>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(directory) = &options.landing_smoke_output else {
        return;
    };
    state.waiting += 1;
    let Some((building, mut instance)) = instances.iter_mut().next() else {
        return;
    };
    if let Some(error) = &instance.error {
        std::fs::create_dir_all(directory).expect("Create landing output");
        std::fs::write(
            directory.join("report.json"),
            serde_json::json!({"asset":options.asset,"errors":[error]}).to_string(),
        )
        .unwrap();
        exit.write(AppExit::error());
        return;
    }
    let Some(bounds) = instance.bounds else {
        if state.waiting == 1800 {
            error!("Landing smoke asset load timed out");
            exit.write(AppExit::error());
        }
        return;
    };
    if instance.clusters.is_empty() {
        return;
    }
    let Some(asset) = assets.get(&instance.asset) else {
        return;
    };
    if state.warmup < 150 {
        state.warmup += 1;
        return;
    }
    state.frame += 1;
    metrics.phase = state.frame;
    for mut camera in &mut cameras {
        let size = bounds.max - bounds.min;
        let center = bounds.center();
        *camera = Transform::from_translation(Vec3::new(
            bounds.max.x + size.y * 0.6,
            size.y * 0.7,
            bounds.max.z + size.y,
        ))
        .looking_at(Vec3::new(center.x, size.y * 0.4, center.z), Vec3::Y);
    }
    let selected_bodies = bodies
        .iter()
        .filter(|body| body.0.building == building)
        .collect::<Vec<_>>();
    if !state.upper.is_empty() {
        let finite = selected_bodies.iter().all(|body| {
            body.1.0.is_finite()
                && body.2.0.is_finite()
                && body.3.0.is_finite()
                && body.4.0.is_finite()
                && body.5.0.is_finite()
        });
        state.finite &= finite;
        if state.first_impact.is_none() && impacts.structural_solves > 0 {
            state.first_impact = Some(state.frame);
            capture(&mut commands, &options, "03-ground-impact.png");
        }
    }
    match state.frame {
        1 => {
            std::fs::create_dir_all(directory).expect("Create landing output");
            state.finite = true;
        }
        10 => capture(&mut commands, &options, "01-intact-tower.png"),
        20 => {
            state.cut = (bounds.min.y + bounds.max.y) * 0.5;
            state.upper = asset
                .manifest
                .chunks
                .iter()
                .filter(|record| {
                    record
                        .transform()
                        .transform_point(record.center_of_mass.into())
                        .y
                        >= state.cut
                })
                .map(|record| record.id.clone())
                .collect();
            if bounds.max.y - bounds.min.y < 12.0 {
                state.errors.push(
                    "Landing validation requires a tall building (use --scene highrise_slab)"
                        .into(),
                );
            }
            state.initial_mass = asset
                .manifest
                .chunks
                .iter()
                .filter(|record| state.upper.contains(&record.id))
                .map(|record| f64::from(record.mass))
                .sum();
            state.initial_bonds = asset
                .manifest
                .bonds
                .iter()
                .filter(|bond| state.upper.contains(&bond.a) && state.upper.contains(&bond.b))
                .count();
            for (id, chunk) in &mut instance.chunks {
                if !state.upper.contains(id) {
                    chunk.alive = false;
                    commands
                        .entity(chunk.entity)
                        .insert((RemovedChunk, Visibility::Hidden))
                        .remove::<FractureEdges>();
                    for &primitive in &chunk.primitives {
                        commands.entity(primitive).despawn();
                    }
                }
            }
            for bond in &asset.manifest.bonds {
                if !state.upper.contains(&bond.a) || !state.upper.contains(&bond.b) {
                    instance.bonds.insert(bond.id.clone(), false);
                }
            }
            instance.topology_dirty = true;
            state.before_impact_broken = impacts.broken_bonds;
        }
        80 => {
            if impacts.broken_bonds != state.before_impact_broken {
                state
                    .errors
                    .push("Unsupported tower fractured before contact".into());
            }
            capture(&mut commands, &options, "02-free-fall.png");
        }
        500 => {
            state.largest_final = selected_bodies
                .iter()
                .map(|body| body.0.chunks.len())
                .max()
                .unwrap_or(0);
            state.internal_broken = asset
                .manifest
                .bonds
                .iter()
                .filter(|bond| {
                    state.upper.contains(&bond.a)
                        && state.upper.contains(&bond.b)
                        && bond.center[1] > state.cut + 4.0
                        && !instance.bonds[&bond.id]
                })
                .count();
            if state.first_impact.is_none() || impacts.structural_solves == 0 {
                state
                    .errors
                    .push("Tower did not undergo a solved structural ground impact".into());
            }
            if state.internal_broken == 0 {
                state.errors.push(
                    "Landing only chipped its contact floor instead of breaking internal storeys"
                        .into(),
                );
            }
            if state.largest_final + 4 >= state.upper.len() {
                state
                    .errors
                    .push("Upper tower remained one giant rigid cluster after impact".into());
            }
            let mass = selected_bodies
                .iter()
                .map(|body| f64::from(body.5.0))
                .sum::<f64>();
            if (mass - state.initial_mass).abs() > state.initial_mass * 1e-5 {
                state
                    .errors
                    .push("Landing fracture changed surviving mass".into());
            }
            if !state.finite {
                state
                    .errors
                    .push("Landing produced a non-finite body state".into());
            }
            if impacts.structural_failures > 0
                || impacts.max_force_balance_ratio > 1e-6
                || impacts.max_moment_balance_ratio > 1e-6
                || impacts.max_gauge_reaction_ratio > 1e-5
            {
                state.errors.push(
                    "Dynamic impact loads or numerical gauge failed equilibrium checks".into(),
                );
            }
            capture(&mut commands, &options, "04-impact-fracture.png");
        }
        530 => {
            for name in [
                "01-intact-tower.png",
                "02-free-fall.png",
                "03-ground-impact.png",
                "04-impact-fracture.png",
            ] {
                if !options.profile && !directory.join(name).is_file() {
                    state.errors.push(format!("Missing screenshot {name}"));
                }
            }
            let report = serde_json::json!({"asset":options.asset,"initial_upper_chunks":state.upper.len(),"initial_upper_bonds":state.initial_bonds,
                "cut_height":state.cut,"initial_upper_mass":state.initial_mass,"first_impact_frame":state.first_impact,
                "largest_final_cluster":state.largest_final,"internal_bonds_broken":state.internal_broken,"finite_body_state":state.finite,
                "impact_broken_bonds":impacts.broken_bonds,"structural_solves":impacts.structural_solves,"structural_failures":impacts.structural_failures,
                "pending_structural":impacts.pending_structural,"max_structural_solve_ms":impacts.max_structural_solve_ms,
                "max_force_balance_ratio":impacts.max_force_balance_ratio,"max_moment_balance_ratio":impacts.max_moment_balance_ratio,
                "max_gauge_reaction_ratio":impacts.max_gauge_reaction_ratio,"metrics":metrics.json(),"errors":state.errors});
            std::fs::write(
                directory.join("report.json"),
                serde_json::to_string_pretty(&report).unwrap(),
            )
            .expect("Write landing report");
            if state.errors.is_empty() {
                info!("Landing smoke passed: {report}");
                exit.write(AppExit::Success);
            } else {
                error!("Landing smoke failed: {report}");
                exit.write(AppExit::error());
            }
        }
        _ => {}
    }
}

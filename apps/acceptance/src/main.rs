use std::{path::PathBuf, time::Duration};

use bevy::{
    app::ScheduleRunnerPlugin, prelude::*, render::RenderPlugin, ui::IsDefaultUiCamera,
    winit::WinitPlugin,
};
use bevy_acceptance_test::{AcceptanceCamera, AcceptancePlugin, OFFSCREEN_IMAGE};
use bevy_destruction::{
    DEFAULT_VIEWPORT, DestructionPlugin,
    acceptance::{AcceptanceScenarioPlugin, DemoCamera, Probe, ProbeRun},
    asset_plugin, parse_options,
};

struct Arguments {
    game: Vec<String>,
    probe: Option<ProbeRun>,
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Arguments, String> {
    let mut game = Vec::new();
    let mut probe = None;
    let mut output = None;
    let mut capture_replay = false;
    let mut profile = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--probe" => {
                let name = args.next().ok_or("--probe requires a name")?;
                probe = Some(name.parse::<Probe>()?);
            }
            "--probe-output" => {
                output = Some(PathBuf::from(
                    args.next().ok_or("--probe-output requires a directory")?,
                ));
            }
            "--asset" | "--scene" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("{arg} requires a value"))?;
                game.extend([arg, value]);
            }
            "--capture-replay" => capture_replay = true,
            "--profile" => profile = true,
            "--help" | "-h" => {
                println!(
                    "bevy-destruction-acceptance [--scene NAME | --asset PATH]\n\
                     [--probe smoke|physics|stress|material|contact|force|landing|collapse --probe-output DIR]\n\
                     [--capture-replay | --profile] [--no-vfx] [--no-stress] [--no-detail] [--no-soft-particles]"
                );
                std::process::exit(0);
            }
            _ => game.push(arg),
        }
    }

    let probe = match (probe, output) {
        (Some(probe), Some(output)) => {
            if (capture_replay || profile) && probe != Probe::Smoke {
                return Err("--capture-replay and --profile require --probe smoke".into());
            }
            if capture_replay && profile {
                return Err("--capture-replay and --profile are mutually exclusive".into());
            }
            Some(ProbeRun {
                probe,
                output,
                capture_replay,
                profile,
            })
        }
        (None, None) if !capture_replay && !profile => None,
        _ => return Err("Provide --probe and --probe-output together".into()),
    };
    Ok(Arguments { game, probe })
}

fn main() -> AppExit {
    let args = parse(std::env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    let mut options = match parse_options(args.game.into_iter()) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            return AppExit::error();
        }
    };
    let acceptance = match AcceptancePlugin::from_env() {
        Ok(acceptance) => acceptance.with_size(DEFAULT_VIEWPORT),
        Err(error) => {
            eprintln!("{error}");
            return AppExit::error();
        }
    };
    let profile = args.probe.as_ref().is_some_and(|run| run.profile);
    let probe = args
        .probe
        .map(|run| run.configure(&mut options, OFFSCREEN_IMAGE));
    let window = acceptance.input_window_plugin();
    let runner = if profile {
        ScheduleRunnerPlugin::run_loop(Duration::ZERO)
    } else {
        acceptance.runner_plugin()
    };

    App::new()
        .register_required_components_with::<DemoCamera, AcceptanceCamera>(|| AcceptanceCamera)
        .register_required_components::<DemoCamera, IsDefaultUiCamera>()
        .add_plugins(acceptance)
        .add_plugins(
            DefaultPlugins
                .build()
                .disable::<WinitPlugin>()
                .set(asset_plugin())
                .set(window)
                .set(RenderPlugin {
                    // Fixed-frame captures need pipelines ready on the frame they request.
                    synchronous_pipeline_compilation: probe.is_some() && !profile,
                    ..default()
                }),
        )
        .add_plugins(runner)
        .insert_resource(options)
        .add_plugins((DestructionPlugin, AcceptanceScenarioPlugin { probe }))
        .run()
}

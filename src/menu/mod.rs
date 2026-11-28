use bevy::{
    core_pipeline::prepass::DepthPrepass,
    picking::{events::PointerClick, pointer::PointerButton},
    prelude::*,
    time::Virtual,
};

use crate::{
    DemoSettings, SETTINGS_APP_NAME,
    camera::{self, CameraControl, FreeCamera},
    load_demo_settings,
    projectiles::{ResetProjectiles, RigidProjectile},
    scene::{self, BuildingInstance, DemoCamera, DemoRoot},
    stress_runtime::StressSettings,
    vfx::{Particle, VfxSettings},
};

pub(crate) struct PauseMenuPlugin;

impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseMenu>()
            .add_message::<MenuAction>()
            .add_systems(Startup, spawn_menu)
            .add_systems(
                Update,
                (complete_reload, read_menu_input, update_menu)
                    .chain()
                    .after(scene::instantiate_pending)
                    .before(camera::initialize_camera),
            );
    }
}

#[derive(Resource, Default)]
pub(crate) struct PauseMenu {
    pub open: bool,
    resume_capture: bool,
    candidate: Option<Entity>,
    status: String,
}

#[derive(Component)]
pub(crate) struct PendingReload {
    pub settings: DemoSettings,
}

#[derive(Component)]
struct MenuRoot;

#[derive(Component)]
struct Status;

#[derive(Clone, Copy, Message)]
enum MenuAction {
    Resume,
    Reload,
    Quit,
}

fn spawn_menu(mut commands: Commands) {
    commands
        .spawn((
            Name::new("Pause menu"),
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100.0),
                height: percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            GlobalZIndex(10),
            BackgroundColor(Color::srgba(0.02, 0.025, 0.035, 0.76)),
            Visibility::Hidden,
        ))
        .with_children(|root| {
            root.spawn((
                Name::new("Pause menu panel"),
                Node {
                    width: px(360.0),
                    padding: UiRect::all(px(24.0)),
                    row_gap: px(12.0),
                    flex_direction: FlexDirection::Column,
                    ..default()
                },
                BackgroundColor(Color::srgb(0.10, 0.12, 0.16)),
            ))
            .with_children(|panel| {
                for (label, action) in [
                    ("Resume", MenuAction::Resume),
                    ("Reload", MenuAction::Reload),
                    ("Quit", MenuAction::Quit),
                ] {
                    panel
                        .spawn((
                            Name::new(label),
                            Node {
                                min_height: px(45.0),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.19, 0.23, 0.30)),
                        ))
                        .observe(
                            move |click: On<PointerClick>, mut actions: MessageWriter<MenuAction>| {
                                if click.button == PointerButton::Primary {
                                    actions.write(action);
                                }
                            },
                        )
                        .with_child((
                            Name::new(format!("{label} label")),
                            Text::new(label),
                            TextFont::from_font_size(24.0),
                        ));
                }
                panel.spawn((
                    Name::new("Reload status"),
                    Status,
                    Text::default(),
                    TextFont::from_font_size(16.0),
                ));
            });
        });
}

fn resume(
    menu: &mut PauseMenu,
    time: &mut Time<Virtual>,
    control: &mut CameraControl,
    mouse: &mut ButtonInput<MouseButton>,
) {
    menu.open = false;
    menu.status.clear();
    time.unpause();
    control.captured = menu.resume_capture;
    mouse.clear_just_pressed(MouseButton::Left);
}

#[allow(clippy::too_many_arguments)]
fn read_menu_input(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut menu: ResMut<PauseMenu>,
    mut time: ResMut<Time<Virtual>>,
    mut control: ResMut<CameraControl>,
    server: Res<AssetServer>,
    root: Single<Entity, With<DemoRoot>>,
    mut actions: MessageReader<MenuAction>,
    mut exit: MessageWriter<AppExit>,
) {
    let clicked = actions.read().last().copied();
    let action = if keys.just_pressed(KeyCode::Escape) {
        if !menu.open {
            menu.open = true;
            menu.resume_capture = control.captured;
            time.pause();
            return;
        }
        Some(MenuAction::Resume)
    } else if menu.open {
        clicked
    } else {
        None
    };

    if matches!(action, Some(MenuAction::Quit)) {
        exit.write(AppExit::Success);
        return;
    }
    if menu.candidate.is_some() {
        return;
    }
    match action {
        Some(MenuAction::Resume) => resume(&mut menu, &mut time, &mut control, &mut mouse),
        Some(MenuAction::Reload) => {
            // SettingsPlugin reads synchronously when built. Use a fresh registry
            // so the loaded settings stay separate until the candidate is ready.
            let mut reader = App::new();
            load_demo_settings(&mut reader, SETTINGS_APP_NAME);
            let settings = reader
                .world_mut()
                .remove_resource::<DemoSettings>()
                .unwrap_or_default();
            let path = match settings.scene_path() {
                Ok(path) => path,
                Err(error) => {
                    menu.status = error.into();
                    return;
                }
            };
            let asset = server.load(path.to_owned());
            menu.candidate = Some(
                commands
                    .spawn((
                        Name::new("BuildingRoot"),
                        Transform::IDENTITY,
                        Visibility::Hidden,
                        BuildingInstance::pending(asset),
                        ChildOf(*root),
                        PendingReload { settings },
                    ))
                    .id(),
            );
            menu.status = "Loading scene…".into();
        }
        None | Some(MenuAction::Quit) => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn complete_reload(
    mut commands: Commands,
    mut menu: ResMut<PauseMenu>,
    mut time: ResMut<Time<Virtual>>,
    mut control: ResMut<CameraControl>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut settings: ResMut<DemoSettings>,
    mut vfx: ResMut<VfxSettings>,
    mut stress: ResMut<StressSettings>,
    candidates: Query<(&BuildingInstance, &PendingReload)>,
    previous: Query<Entity, (With<BuildingInstance>, Without<PendingReload>)>,
    projectiles: Query<Entity, With<RigidProjectile>>,
    mut cameras: Query<(Entity, &mut FreeCamera), With<DemoCamera>>,
    mut particles: Query<&mut Particle>,
) {
    let Some(candidate) = menu.candidate else {
        return;
    };
    let Ok((building, pending)) = candidates.get(candidate) else {
        return;
    };
    if let Some(error) = &building.error {
        menu.status.clone_from(error);
        menu.candidate = None;
        commands.entity(candidate).despawn();
        return;
    }
    if building.bounds.is_none() {
        return;
    }

    for entity in previous.iter().chain(projectiles.iter()) {
        commands.entity(entity).despawn();
    }
    for mut particle in &mut particles {
        particle.age = particle.lifetime;
    }
    commands.trigger(ResetProjectiles);

    *settings = pending.settings.clone();
    vfx.enabled = settings.vfx;
    stress.enabled = settings.stress;
    for (entity, mut camera) in &mut cameras {
        *camera = FreeCamera::default();
        if settings.soft_particles {
            commands.entity(entity).insert(DepthPrepass);
        } else {
            commands.entity(entity).remove::<DepthPrepass>();
        }
    }
    commands
        .entity(candidate)
        .remove::<PendingReload>()
        .insert(Visibility::Visible);
    menu.candidate = None;
    resume(&mut menu, &mut time, &mut control, &mut mouse);
}

fn update_menu(
    menu: Res<PauseMenu>,
    mut root: Single<&mut Visibility, With<MenuRoot>>,
    mut status: Single<&mut Text, With<Status>>,
) {
    if menu.is_changed() {
        **root = if menu.open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        status.0.clone_from(&menu.status);
    }
}

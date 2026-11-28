use bevy::{core_pipeline::prepass::DepthPrepass, prelude::*};

use crate::{
    camera::{Crosshair, FreeCamera},
    menu::PendingReload,
    settings::DemoSettings,
};

use super::{BuildingInstance, ChunkInstance, DemoCamera, DemoRoot};

#[derive(Component)]
pub(super) struct Hint;

pub(super) fn setup(mut commands: Commands, server: Res<AssetServer>, settings: Res<DemoSettings>) {
    let root = commands
        .spawn_scene(bsn! { Name::new("DemoRoot") Transform::IDENTITY Visibility::Visible })
        .insert(DemoRoot)
        .id();
    let camera = commands.spawn_scene(bsn! {
        Name::new("Free flight camera")
        Camera3d::default()
        Projection::from(PerspectiveProjection { fov: crate::camera::FIELD_OF_VIEW, near: 0.05, far: 500.0, ..default() })
        Transform::from_xyz(0.0, 3.0, 30.0)
        AmbientLight { brightness: 280.0 }
    }).insert((DemoCamera, FreeCamera::default(), Msaa::Off, ChildOf(root))).id();
    if settings.soft_particles {
        commands.entity(camera).insert(DepthPrepass);
    }
    commands.spawn_scene(bsn! {
        Name::new("Crosshair")
        Node { position_type: PositionType::Absolute, width: percent(100.0), height: percent(100.0), justify_content: JustifyContent::Center, align_items: AlignItems::Center }
        Visibility::Visible
        Children [
            Name::new("Crosshair glyph")
            Text::new("+")
            TextFont { font_size: px(24.0) }
            TextColor(Color::srgb(0.95, 0.7, 0.3))
        ]
    }).insert(Crosshair);
    commands
        .spawn_scene(bsn! {
            Name::new("Key light")
            DirectionalLight { illuminance: 6500.0, shadow_maps_enabled: true }
            Transform::from_xyz(-10.0, 14.0, 12.0).looking_at(Vec3::ZERO, Vec3::Y)
        })
        .insert(ChildOf(root));
    let building = match settings.scene_path() {
        Ok(path) => BuildingInstance::pending(server.load(path.to_owned())),
        Err(error) => {
            let mut building = BuildingInstance::pending(Handle::default());
            building.error = Some(error.into());
            building
        }
    };
    commands
        .spawn_scene(bsn! { Name::new("BuildingRoot") Transform::IDENTITY Visibility::Visible })
        .insert((building, ChildOf(root)));
    commands
        .spawn_scene(bsn! {
            Name::new("Building status")
            Text::new("Loading building…")
            TextFont { font_size: px(19.0) }
            TextColor(Color::srgb(0.88, 0.9, 0.94))
            Node { position_type: PositionType::Absolute, left: px(22.0), bottom: px(18.0) }
        })
        .insert(Hint);
}

pub(super) fn update_hint(
    instances: Query<
        (
            &BuildingInstance,
            Option<&crate::stress_runtime::StressState>,
        ),
        Without<PendingReload>,
    >,
    chunks: Query<&ChunkInstance>,
    mut hints: Query<&mut Text, With<Hint>>,
) {
    let Some((instance, stress)) = instances.iter().next() else {
        return;
    };
    let mut text = if let Some(error) = &instance.error {
        format!("Building error: {error}")
    } else if instance.bounds.is_some() {
        format!(
            "WASD: move | Mouse: look | Space/Ctrl: up/down | Tab: cursor\nLeft: break | Right/R: rigid ball/cube | F1-F6: scenes | 0: edges | 9: heat | Esc: exit     {}/{} chunks",
            instance
                .chunks
                .values()
                .filter(|&&entity| chunks.get(entity).is_ok_and(|chunk| chunk.alive))
                .count(),
            instance.chunks.len()
        )
    } else {
        "Loading building…".into()
    };
    if let Some(stress) = stress {
        let invalid = stress.reports.iter().filter(|report| !report.valid).count();
        let max_displacement = stress
            .reports
            .iter()
            .filter(|report| report.valid)
            .map(|report| report.max_displacement)
            .fold(0.0_f64, f64::max);
        if invalid > 0 && invalid == stress.reports.len() {
            text.push_str(&format!(
                "\nStress: unavailable (retrying) | {} overload breaks | {invalid} unsolved components",
                stress.broken_total
            ));
        } else {
            text.push_str(&format!("\nStress: {:.2} capacity | {} overload breaks | {} unsolved components | {:.3} mm elastic estimate",
                stress.max_utilization, stress.broken_total, invalid, max_displacement * 1000.0));
        }
    }
    for mut hint in &mut hints {
        if hint.0 != text {
            hint.0 = text.clone();
        }
    }
}

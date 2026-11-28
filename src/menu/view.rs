use bevy::{
    picking::{events::PointerClick, pointer::PointerButton},
    prelude::*,
};

use super::{MenuAction, PauseMenu};

#[derive(Component)]
pub(super) struct MenuRoot;

#[derive(Component)]
pub(super) struct Status;

pub(super) fn spawn_menu(mut commands: Commands) {
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

pub(super) fn update_menu(
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

use std::fmt::Write;

use bevy::prelude::*;

use crate::force_samples::ForceCapture;

use super::{ForceGizmos, ForceInspection, ForceViewStats};

#[derive(Component)]
pub(super) enum ForceHud {
    Legend,
    Inspection,
}

pub(super) fn setup(mut commands: Commands, mut configs: ResMut<GizmoConfigStore>) {
    let (config, _) = configs.config_mut::<ForceGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 2.5;
    for (name, panel, position) in [
        (
            "Force legend",
            ForceHud::Legend,
            Node {
                left: px(18.0),
                top: px(18.0),
                ..default()
            },
        ),
        (
            "Force inspection",
            ForceHud::Inspection,
            Node {
                right: px(18.0),
                top: px(62.0),
                width: px(310.0),
                max_width: percent(55.0),
                ..default()
            },
        ),
    ] {
        commands.spawn((
            Name::new(name),
            panel,
            Text::new(""),
            TextFont {
                font_size: px(16.0).into(),
                ..default()
            },
            TextColor(Color::srgb(0.94, 0.96, 1.0)),
            BackgroundColor(Color::srgba(0.015, 0.02, 0.03, 0.88)),
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                padding: UiRect::all(px(10.0)),
                ..position
            },
            ZIndex(30),
        ));
    }
}

fn quantity(value: f64, unit: &str) -> String {
    if value >= 1e6 {
        format!("{:.2} M{unit}", value / 1e6)
    } else if value >= 1e3 {
        format!("{:.2} k{unit}", value / 1e3)
    } else {
        format!("{value:.2} {unit}")
    }
}

pub(super) fn inspection_text(inspection: &ForceInspection) -> String {
    let mut text = format!("Chunk {}", inspection.chunk.1);
    if inspection.bonds.is_empty() {
        text.push_str("\nNo valid connection loads");
    }
    for (index, bond) in inspection.bonds.iter().enumerate() {
        let label = if bond.impact {
            "Impact J/10 ms"
        } else {
            "Static"
        };
        let _ = write!(
            text,
            "\n{}  U {:.1}%  {}\nF {} | M {}",
            index + 1,
            100.0 * bond.utilization,
            label,
            quantity(bond.force_n, "N"),
            quantity(bond.moment_nm, "N*m"),
        );
    }
    if let Some(impulse) = inspection.impulse_ns {
        let _ = write!(text, "\nContact J {}", quantity(impulse, "N*s"));
    }
    text
}

pub(super) fn update_hud(
    capture: Res<ForceCapture>,
    stats: Res<ForceViewStats>,
    mut hud: Query<(&ForceHud, &mut Text, &mut Node)>,
) {
    for (panel, mut text, mut node) in &mut hud {
        let content = match panel {
            ForceHud::Legend if capture.enabled => {
                Some("9: LOAD HEAT | Amber 50% to <100% | Red >=100% | Hold Alt to inspect".into())
            }
            ForceHud::Inspection => stats.inspected.as_ref().map(inspection_text),
            _ => None,
        };
        let display = if content.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if node.display != display {
            node.display = display;
        }
        if let Some(content) = content
            && text.0 != content
        {
            text.0 = content;
        }
    }
}

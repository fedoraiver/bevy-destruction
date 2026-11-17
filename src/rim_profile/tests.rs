use super::*;

#[test]
fn profiles_are_stable_and_preserve_shape_when_the_edge_reverses() {
    let start = Vec3::new(-1.0, 4.0, 2.0);
    let end = Vec3::new(2.0, 4.0, 2.0);
    let profile = rim_profile("patch", start, end);
    assert_eq!(profile, rim_profile("patch", start, end));
    assert_ne!(profile, rim_profile("other", start, end));
    let reverse = rim_profile("patch", end, start);
    assert_eq!(profile.len(), reverse.len());
    for (a, b) in profile.iter().zip(reverse.iter().rev()) {
        assert!((a.t + b.t - 1.0).abs() < 1e-6);
        assert_eq!((a.height, a.width, a.embed), (b.height, b.width, b.embed));
    }
}

#[test]
fn knots_respect_spacing_depth_and_geometry_budgets_at_every_scale() {
    for length in [0.0, 0.01, 0.035, 0.1, 0.4, 1.0, 3.0, 12.0] {
        for seed in 0..32 {
            let profile = rim_profile(&format!("patch-{seed}"), Vec3::ZERO, Vec3::X * length);
            assert_eq!(profile.first().unwrap().t, 0.0);
            assert_eq!(profile.last().unwrap().t, 1.0);
            assert_eq!(profile.first().unwrap().height, 0.0);
            assert_eq!(profile.last().unwrap().height, 0.0);
            assert!(profile.len() <= (length * KNOTS_PER_METER).ceil() as usize + 2);
            assert!(profile.iter().all(|point| {
                point.t.is_finite()
                    && (0.0..=1.0).contains(&point.height)
                    && (0.8..=2.2).contains(&point.width)
                    && (0.16..=0.4).contains(&point.embed)
            }));
            for pair in profile.windows(2) {
                assert!(pair[1].t > pair[0].t);
                if profile.len() > 2 {
                    assert!((pair[1].t - pair[0].t) * length >= MIN_KNOT_SPACING - 1e-6);
                }
            }
        }
    }
}

#[test]
fn silhouette_has_quiet_spans_clustered_shoulders_and_varied_spacing() {
    let profile = rim_profile("concrete-breakout", Vec3::ZERO, Vec3::X * 3.0);
    let spacings: Vec<_> = profile
        .windows(2)
        .map(|pair| pair[1].t - pair[0].t)
        .collect();
    let minimum = spacings.iter().copied().fold(f32::INFINITY, f32::min);
    let maximum = spacings.iter().copied().fold(0.0_f32, f32::max);
    assert!(maximum / minimum > 3.0);
    assert!(profile.windows(2).any(|pair| {
        pair[0].height == 0.0 && pair[1].height == 0.0 && (pair[1].t - pair[0].t) * 3.0 > 0.035
    }));
    assert!(
        profile
            .windows(2)
            .any(|pair| pair[0].height > 0.4 && pair[1].height > 0.4)
    );
    assert!(profile.iter().any(|point| point.height > 0.75));
}

#[test]
fn irregular_edges_retain_visible_chip_coverage() {
    let mean_height = (0..32)
        .map(|seed| {
            rim_profile(&format!("coverage-{seed}"), Vec3::ZERO, Vec3::X)
                .windows(2)
                .map(|pair| (pair[1].t - pair[0].t) * (pair[0].height + pair[1].height) * 0.5)
                .sum::<f32>()
        })
        .sum::<f32>()
        / 32.0;
    assert!(
        mean_height > 0.25,
        "Irregular rims became almost flat: {mean_height}"
    );
}

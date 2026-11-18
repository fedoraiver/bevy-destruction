use super::*;

#[test]
fn bounded_history_replaces_a_body_and_expires_invalid_members() {
    let mut world = World::new();
    let building = world.spawn_empty().id();
    let bodies = (0..12)
        .map(|_| world.spawn_empty().id())
        .collect::<Vec<_>>();
    let mut samples = ForceSamples::default();
    for &body in &bodies {
        samples.record_force(DynamicForceSample {
            building,
            body,
            chunks: vec!["a".into(), "b".into()],
            bonds: BTreeMap::new(),
            bond_signature: 0,
            captured_at: 1.0,
        });
    }
    assert_eq!(samples.dynamic.len(), MAX_FORCE_SAMPLES);
    samples.record_force(DynamicForceSample {
        building,
        body: bodies[11],
        chunks: vec!["a".into(), "b".into()],
        bonds: BTreeMap::new(),
        bond_signature: 0,
        captured_at: 1.2,
    });
    assert_eq!(samples.dynamic.len(), MAX_FORCE_SAMPLES);
    for _ in 0..150 {
        samples.record_impulse(ImpulseSample {
            building,
            chunk: "a".into(),
            impulse: Vec3::Y,
            captured_at: 1.0,
        });
    }
    assert_eq!(samples.impulses.len(), MAX_IMPULSE_SAMPLES);
    samples.prune(1.5, |sample| sample.body == bodies[11], |_| false);
    assert_eq!(samples.dynamic.len(), 1);
    assert!(samples.impulses.is_empty());
    samples.prune(2.0, |_| true, |_| true);
    assert!(samples.dynamic.is_empty());
}

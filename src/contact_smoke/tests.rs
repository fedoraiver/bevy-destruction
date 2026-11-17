use super::*;

#[test]
fn matrix_regions_cover_contact_grid_and_forward_occluder_in_every_pose() {
    for case in cases() {
        let r = regions(case);
        assert!(r.contact.len() > 64, "{} contact", case.label());
        assert!(r.grid.len() > 64, "{} grid", case.label());
        assert!(r.occluder.len() > 64, "{} occluder", case.label());
        let occluder = r
            .occluder
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert!(r.contact.iter().all(|i| !occluder.contains(i)));
    }
}

#[test]
fn pixel_comparison_rejects_grid_leaks_exterior_paint_and_occluder_penetration() {
    let r = regions(cases()[0]);
    let reference = [120, 100, 80, 255].repeat(WIDTH * HEIGHT);
    assert!(!failed(&difference(&reference, &reference, &r.contact)));
    for indices in [&r.contact, &r.grid, &r.occluder] {
        let mut wrong = reference.clone();
        for &i in indices.iter().step_by(4) {
            wrong[i..i + 3].copy_from_slice(&[185, 190, 195]);
        }
        assert!(failed(&difference(&reference, &wrong, indices)));
    }
}

#[test]
fn phase_witness_rejects_black_and_stale_readbacks() {
    let mut pixels = vec![0; WIDTH * HEIGHT * 4];
    assert_eq!(witness(&pixels), 0);
    let phase_code = 17;
    for bit in 0..6 {
        if phase_code & (1 << bit) == 0 {
            continue;
        }
        let x = 20 + bit * 12;
        for y in 19..=21 {
            for x in x - 1..=x + 1 {
                let i = (y * WIDTH + x) * 4;
                pixels[i..i + 3].fill(255);
            }
        }
    }
    assert_eq!(witness(&pixels), phase_code);
    assert_ne!(witness(&pixels), phase_code + 1);
}

#[test]
fn source_and_reference_preserve_actual_gltf_double_sided_material_state() {
    let imported = GltfMaterial {
        double_sided: true,
        cull_mode: None,
        ..default()
    };
    for fracture in [false, true] {
        let material = building_material(Some(&imported), fracture, "concrete");
        assert!(material.base.double_sided);
        assert!(material.base.cull_mode.is_none());
    }
}

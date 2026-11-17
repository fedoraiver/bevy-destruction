use super::*;

    #[test]
    fn material_keeps_gltf_double_sided_and_uses_material_class_for_fracture() {
        let source = GltfMaterial {
            double_sided: true,
            ..default()
        };
        let steel = building_material(Some(&source), true, "steel");
        let concrete = building_material(Some(&source), true, "concrete");
        assert!(steel.base.double_sided);
        assert!(steel.base.metallic > concrete.base.metallic);
        assert!(concrete.extension.settings.fracture.y > 0.0);
        assert_ne!(
            steel.extension.settings.fracture_color,
            concrete.extension.settings.fracture_color
        );
    }

    fn positions(mesh: &Mesh) -> &Vec<[f32; 3]> {
        let Some(VertexAttributeValues::Float32x3(values)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("float3 positions");
        };
        values
    }

    fn coordinates(mesh: &Mesh) -> Vec<Vec3> {
        let Some(VertexAttributeValues::Float32x2(xy)) = mesh.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!("surface xy coordinates");
        };
        let Some(VertexAttributeValues::Float32x2(z)) = mesh.attribute(Mesh::ATTRIBUTE_UV_1) else {
            panic!("surface z coordinates");
        };
        xy.iter()
            .zip(z)
            .map(|(xy, z)| Vec3::new(xy[0], xy[1], z[0]))
            .collect()
    }

    #[test]
    fn attached_coordinates_share_phase_across_chunks_and_cache_without_mutating_source() {
        let mut meshes = Assets::<Mesh>::default();
        let mut cache = SurfaceMeshCache::default();
        let source = meshes.add(Cuboid::from_length(1.0));
        let original_uv = meshes
            .get(&source)
            .unwrap()
            .attribute(Mesh::ATTRIBUTE_UV_0)
            .cloned();
        let rest_a = Transform::from_xyz(0.5, 0.0, 0.0);
        let rest_b = Transform::from_xyz(1.5, 0.0, 0.0);
        let a = cache.get_or_create(&source, rest_a, &mut meshes);
        let b = cache.get_or_create(&source, rest_b, &mut meshes);
        assert_eq!(a, cache.get_or_create(&source, rest_a, &mut meshes));
        assert_ne!(a, b);
        assert_eq!(meshes.len(), 3);
        assert_eq!(
            meshes.get(&source).unwrap().attribute(Mesh::ATTRIBUTE_UV_0),
            original_uv.as_ref()
        );
        let on_seam = |mesh: &Mesh| {
            coordinates(mesh)
                .into_iter()
                .filter(|point| (point.x - 1.0).abs() < 1e-6)
                .collect::<Vec<_>>()
        };
        let a = on_seam(meshes.get(&a).unwrap());
        let b = on_seam(meshes.get(&b).unwrap());
        assert!(!a.is_empty());
        assert!(a.iter().all(|point| b.contains(point)));
    }

    #[test]
    fn rim_rest_coordinates_survive_local_conversion_and_independent_rigid_motion() {
        let rest = Mesh::from(Cuboid::from_length(1.0));
        let expected = positions(&rest)
            .iter()
            .copied()
            .map(Vec3::from_array)
            .collect::<Vec<_>>();
        let chunk = Transform::from_xyz(4.13, 2.71, -1.37).with_rotation(Quat::from_euler(
            EulerRot::XYZ,
            0.2,
            0.6,
            -0.3,
        ));
        let local = Transform::from_matrix(chunk.to_matrix().inverse());
        let mesh = surface_coordinates(rest, Transform::IDENTITY).transformed_by(local);
        assert_eq!(coordinates(&mesh), expected);
        for motion in [
            Transform::IDENTITY,
            Transform::from_xyz(7.4, -1.3, 2.6).with_rotation(Quat::from_euler(
                EulerRot::XYZ,
                0.8,
                -0.4,
                0.9,
            )),
        ] {
            for ((vertex, coordinate), expected) in positions(&mesh)
                .iter()
                .zip(coordinates(&mesh))
                .zip(&expected)
            {
                let world =
                    motion.transform_point(chunk.transform_point(Vec3::from_array(*vertex)));
                assert!(world.distance(motion.transform_point(*expected)) < 1e-5);
                assert_eq!(coordinate, *expected);
            }
        }
    }

    #[test]
    fn unloaded_source_releases_cached_copies_without_removing_live_meshes() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_resource::<SurfaceMeshCache>()
            .add_systems(Update, release_surface_meshes);
        app.finish();
        app.cleanup();
        let source = app
            .world_mut()
            .resource_mut::<Assets<Mesh>>()
            .add(Cuboid::from_length(1.0));
        let (live, cached_id) =
            app.world_mut()
                .resource_scope(|world, mut cache: Mut<SurfaceMeshCache>| {
                    let mut meshes = world.resource_mut::<Assets<Mesh>>();
                    let live = cache.get_or_create(&source, Transform::IDENTITY, &mut meshes);
                    let cached = cache.get_or_create(
                        &source,
                        Transform::from_xyz(1.0, 0.0, 0.0),
                        &mut meshes,
                    );
                    (live, cached.id())
                });
        let entity = app.world_mut().spawn(Mesh3d(live.clone())).id();
        assert_eq!(app.world().resource::<SurfaceMeshCache>().0.len(), 2);
        app.world_mut()
            .resource_mut::<Assets<Mesh>>()
            .remove(source.id());
        for _ in 0..4 {
            app.update();
        }
        assert!(app.world().resource::<SurfaceMeshCache>().0.is_empty());
        assert!(app.world().resource::<Assets<Mesh>>().get(&live).is_some());
        assert!(
            app.world()
                .resource::<Assets<Mesh>>()
                .get(cached_id)
                .is_none()
        );
        drop(live);
        app.world_mut().despawn(entity);
        for _ in 0..4 {
            app.update();
        }
        assert!(app.world().resource::<Assets<Mesh>>().is_empty());
    }

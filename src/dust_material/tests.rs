use super::*;

    #[test]
    fn atlas_has_distinct_animated_shapes_and_transparent_borders() {
        let image = dust_atlas();
        let data = image.data.as_ref().unwrap();
        let width = (TILE_SIZE * DUST_FRAMES) as usize;
        for row in 0..DUST_VARIANTS as usize {
            for frame in 0..DUST_FRAMES as usize {
                for (x, y) in [(0, 0), (63, 0), (0, 63), (63, 63)] {
                    assert_eq!(data[((row * 64 + y) * width + frame * 64 + x) * 4 + 3], 0);
                }
            }
        }
        let sample = |frame: usize, variant: usize| {
            (0..64)
                .flat_map(|y| {
                    (0..64).map(move |x| ((variant * 64 + y) * width + frame * 64 + x) * 4 + 3)
                })
                .map(|index| data[index])
                .collect::<Vec<_>>()
        };
        assert_ne!(sample(0, 0), sample(7, 0));
        assert_ne!(sample(3, 0), sample(3, 1));
    }

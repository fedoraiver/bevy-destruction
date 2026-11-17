use bevy::prelude::Vec3;

const MIN_KNOT_SPACING: f32 = 0.012;
const KNOTS_PER_METER: f32 = 28.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RimSample {
    pub t: f32,
    pub height: f32,
    pub width: f32,
    pub embed: f32,
}

struct Random(u64);

impl Random {
    fn unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^= value >> 31;
        (value >> 40) as f32 / 16777216.0
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }
}

fn sample(distance: f32, height: f32) -> RimSample {
    RimSample {
        t: distance,
        height,
        width: 1.0,
        embed: 0.22,
    }
}

fn push(samples: &mut Vec<RimSample>, next: RimSample) {
    if next.t - samples.last().unwrap().t >= MIN_KNOT_SPACING {
        samples.push(next);
    }
}

fn reduce(samples: &mut Vec<RimSample>, limit: usize) {
    while samples.len() > limit {
        let least_visible = (1..samples.len() - 1)
            .min_by(|&a, &b| {
                let deviation = |index: usize| {
                    let [previous, current, next] =
                        [samples[index - 1], samples[index], samples[index + 1]];
                    let along = (current.t - previous.t) / (next.t - previous.t);
                    let linear = |a: f32, b: f32| a + (b - a) * along;
                    ((current.height - linear(previous.height, next.height)).abs()
                        + (current.width - linear(previous.width, next.width)).abs() * 0.08
                        + (current.embed - linear(previous.embed, next.embed)).abs() * 0.1)
                        * (next.t - previous.t)
                };
                deviation(a).total_cmp(&deviation(b))
            })
            .unwrap();
        samples.remove(least_visible);
    }
}

fn channel(samples: &[RimSample], seed: u64, range: [f32; 2], span: [f32; 2]) -> Vec<f32> {
    let mut random = Random(seed);
    let mut start = 0.0;
    let mut end = random.range(span[0], span[1]);
    let mut first = random.range(range[0], range[1]);
    let mut last = random.range(range[0], range[1]);
    samples
        .iter()
        .map(|sample| {
            while sample.t > end {
                start = end;
                end += random.range(span[0], span[1]);
                first = last;
                last = random.range(range[0], range[1]);
            }
            first + (last - first) * ((sample.t - start) / (end - start))
        })
        .collect()
}

/// Angular breakouts are grouped between quiet spans; dimensions are fractions of the
/// caller's wall-thickness-limited depth. Endpoint reversal preserves the same profile.
pub fn rim_profile(id: &str, start: Vec3, end: Vec3) -> Vec<RimSample> {
    let reversed = start
        .to_array()
        .into_iter()
        .zip(end.to_array())
        .map(|(a, b)| a.total_cmp(&b))
        .find(|order| !order.is_eq())
        .is_some_and(|order| order.is_gt());
    let (first, last) = if reversed { (end, start) } else { (start, end) };
    let mut seed = 0xcbf29ce484222325_u64;
    for byte in id.bytes().chain(
        first
            .to_array()
            .into_iter()
            .chain(last.to_array())
            .flat_map(|value| ((value * 10000.0).round() as i64).to_le_bytes()),
    ) {
        seed = (seed ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    let mut random = Random(seed);
    let length = first.distance(last);
    let mut samples = vec![sample(0.0, 0.0)];
    let mut distance = 0.0;
    while length - distance > 0.075 {
        // Quiet sections can remain straight for a while instead of putting a tooth
        // between every pair of sample rings.
        let quiet = random.range(0.025, 0.12).min(length - distance - 0.04);
        distance += quiet;
        push(&mut samples, sample(distance, 0.0));
        let count = 1 + (random.unit() * 3.0) as usize;
        let mut strength = random.range(0.65, 1.0);
        for chip in 0..count {
            let microchip = random.unit() < 0.24;
            let width = if microchip {
                random.range(0.035, 0.075)
            } else {
                random.range(0.085, 0.28)
            };
            if distance + width > length - MIN_KNOT_SPACING {
                break;
            }
            strength = (strength * random.range(0.8, 1.22)).clamp(0.55, 1.0);
            let height = if microchip {
                strength * random.range(0.3, 0.65)
            } else {
                strength
            };
            let peak = random
                .range(0.24, 0.76)
                .clamp(MIN_KNOT_SPACING / width, 1.0 - MIN_KNOT_SPACING / width);
            push(&mut samples, sample(distance + width * peak, height));
            if width > 0.11 && random.unit() < 0.85 {
                let shoulder = distance + width * random.range(peak + 0.08, 0.94);
                if distance + width - shoulder >= MIN_KNOT_SPACING {
                    let shoulder_height = height * random.range(0.65, 0.98);
                    push(&mut samples, sample(shoulder, shoulder_height));
                }
            }
            distance += width;
            let neck = if chip + 1 < count {
                height * random.range(0.35, 0.8)
            } else {
                0.0
            };
            push(&mut samples, sample(distance, neck));
        }
    }
    while samples.len() > 1 && length - samples.last().unwrap().t < MIN_KNOT_SPACING {
        samples.pop();
    }
    samples.push(sample(length, 0.0));
    let widths = channel(
        &samples,
        seed ^ 0x8f3e17d209a465b1,
        [0.8, 2.2],
        [0.15, 0.42],
    );
    let embeds = channel(
        &samples,
        seed ^ 0x41b7e8ac35c6920d,
        [0.16, 0.4],
        [0.22, 0.55],
    );
    for ((point, width), embed) in samples.iter_mut().zip(widths).zip(embeds) {
        point.width = width;
        point.embed = embed;
    }
    reduce(&mut samples, (length * KNOTS_PER_METER).ceil() as usize + 2);
    for point in &mut samples {
        point.t = if length > 0.0 { point.t / length } else { 0.0 };
    }
    samples.last_mut().unwrap().t = 1.0;
    if reversed {
        samples.reverse();
        for point in &mut samples {
            point.t = 1.0 - point.t;
        }
    }
    samples
}

#[cfg(test)]
mod tests {
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
}

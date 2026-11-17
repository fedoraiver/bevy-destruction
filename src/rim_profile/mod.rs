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
mod tests;

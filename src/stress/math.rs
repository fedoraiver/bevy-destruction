pub(super) type V3 = [f64; 3];
pub(super) type V6 = [f64; 6];
pub(super) type M3 = [[f64; 3]; 3];
pub(super) type M6 = [[f64; 6]; 6];

pub(super) fn dot(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}
pub(super) fn dot6(a: &V6, b: &V6) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3] + a[4] * b[4] + a[5] * b[5]
}
pub(super) fn add3(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] + b[i])
}
pub(super) fn sub3(a: V3, b: V3) -> V3 {
    std::array::from_fn(|i| a[i] - b[i])
}
pub(super) fn scale3(a: V3, k: f64) -> V3 {
    a.map(|a| a * k)
}
pub(super) fn dot3(a: V3, b: V3) -> f64 {
    dot(&a, &b)
}
pub(super) fn norm3(a: V3) -> f64 {
    dot3(a, a).sqrt()
}
pub(super) fn cross3(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(super) fn identity3() -> M3 {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}
pub(super) fn outer3(a: V3, b: V3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| a[i] * b[j]))
}
pub(super) fn skew3([x, y, z]: V3) -> M3 {
    [[0.0, -z, y], [z, 0.0, -x], [-y, x, 0.0]]
}
pub(super) fn mul3(a: M3, x: V3) -> V3 {
    a.map(|row| dot3(row, x))
}
pub(super) fn addm3(a: M3, b: M3) -> M3 {
    std::array::from_fn(|i| add3(a[i], b[i]))
}
pub(super) fn subm3(a: M3, b: M3) -> M3 {
    std::array::from_fn(|i| sub3(a[i], b[i]))
}
pub(super) fn scale_m3(a: M3, k: f64) -> M3 {
    a.map(|row| scale3(row, k))
}
pub(super) fn transpose3(a: M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| a[j][i]))
}
pub(super) fn mulm3(a: M3, b: M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}
pub(super) fn mul6(a: M6, b: V6) -> V6 {
    a.map(|row| dot(&row, &b))
}
pub(super) fn transpose_mul_vec6(a: M6, b: V6) -> V6 {
    std::array::from_fn(|i| (0..6).map(|j| a[j][i] * b[j]).sum())
}
pub(super) fn mulm6(a: M6, b: M6) -> M6 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..6).map(|k| a[i][k] * b[k][j]).sum()))
}
pub(super) fn transpose_mul6(a: M6, b: M6) -> M6 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..6).map(|k| a[k][i] * b[k][j]).sum()))
}
pub(super) fn add_m6(a: &mut M6, b: M6) {
    for i in 0..6 {
        for j in 0..6 {
            a[i][j] += b[i][j];
        }
    }
}

pub(super) fn jacobian(offset: V3, sign: f64, length_scale: f64) -> M6 {
    let mut matrix = [[0.0; 6]; 6];
    let skew = skew3(offset);
    for i in 0..3 {
        matrix[i][i] = sign;
        matrix[i + 3][i + 3] = sign / length_scale;
        for j in 0..3 {
            matrix[i][j + 3] = -sign * skew[i][j] / length_scale;
        }
    }
    matrix
}

#[derive(Clone, Copy, Default)]
pub(super) struct Compensated {
    pub(super) high: f64,
    pub(super) low: f64,
}

impl Compensated {
    pub(super) fn add(&mut self, value: f64) {
        let sum = self.high + value;
        let virtual_value = sum - self.high;
        let error = (self.high - (sum - virtual_value)) + (value - virtual_value);
        let tail = self.low + error;
        self.high = sum + tail;
        self.low = tail - (self.high - sum);
    }

    pub(super) fn product(&mut self, a: f64, b: f64) {
        let product = a * b;
        self.add(product);
        self.add(a.mul_add(b, -product));
    }

    pub(super) fn value(self) -> f64 {
        self.high + self.low
    }
}

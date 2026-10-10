use kraken_proto::Point;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    a: [f64; 6],
}

impl Affine {
    pub fn fit(pairs: &[(Point, Point)]) -> Option<Self> {
        if pairs.len() < 3 {
            return None;
        }
        let mut m = [[0f64; 3]; 3];
        let mut bx = [0f64; 3];
        let mut by = [0f64; 3];
        for (src, dst) in pairs {
            let v = [f64::from(src.x), f64::from(src.y), 1.0];
            for r in 0..3 {
                for c in 0..3 {
                    m[r][c] += v[r] * v[c];
                }
                bx[r] += v[r] * f64::from(dst.x);
                by[r] += v[r] * f64::from(dst.y);
            }
        }
        let inv = invert(m)?;
        let solve = |b: [f64; 3]| -> [f64; 3] {
            [0, 1, 2].map(|r| inv[r][0] * b[0] + inv[r][1] * b[1] + inv[r][2] * b[2])
        };
        let x = solve(bx);
        let y = solve(by);
        Some(Self {
            a: [x[0], x[1], x[2], y[0], y[1], y[2]],
        })
    }

    pub fn apply(&self, p: Point) -> Point {
        let (x, y) = (f64::from(p.x), f64::from(p.y));
        Point::new(
            (self.a[0] * x + self.a[1] * y + self.a[2]) as f32,
            (self.a[3] * x + self.a[4] * y + self.a[5]) as f32,
        )
    }
}

fn invert(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    if det.abs() < 1e-9 {
        return None;
    }
    let d = 1.0 / det;
    Some([
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d,
        ],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_isometric_transform() {
        let truth = |p: Point| Point::new(0.5 * p.x - 0.5 * p.y + 3.0, 0.25 * p.x + 0.25 * p.y - 7.0);
        let pairs: Vec<_> = [(0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (37.0, -12.0)]
            .map(|(x, y)| Point::new(x, y))
            .map(|p| (p, truth(p)))
            .to_vec();
        let fit = Affine::fit(&pairs).unwrap();
        let probe = Point::new(123.0, 45.0);
        assert!(fit.apply(probe).distance(truth(probe)) < 1e-3);
    }

    #[test]
    fn rejects_degenerate_input() {
        let line: Vec<_> = (0..5)
            .map(|i| (Point::new(i as f32, i as f32), Point::new(0.0, 0.0)))
            .collect();
        assert!(Affine::fit(&line).is_none());
    }
}

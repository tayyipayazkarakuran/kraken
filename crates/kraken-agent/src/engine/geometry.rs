use kraken_proto::Point;

#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    half_w: f32,
    half_h: f32,
}

impl Geometry {
    pub fn new(width: f32, height: f32) -> Option<Self> {
        (width > 0.0 && height > 0.0).then_some(Self {
            half_w: width / 2.0,
            half_h: height / 2.0,
        })
    }

    pub fn to_uv(self, p: Point) -> (f32, f32) {
        ((p.x - self.half_w) / self.half_w, p.y / self.half_h)
    }

    pub fn uv_point(self, u: f32, v: f32) -> Point {
        Point::new(self.half_w + u * self.half_w, v * self.half_h)
    }

    pub fn centre(self) -> Point {
        self.uv_point(0.0, 0.0)
    }

    pub fn contains(self, p: Point, margin: f32) -> bool {
        let (u, v) = self.to_uv(p);
        u.abs() + v.abs() <= 1.0 - margin
    }

    pub fn clamp(self, p: Point, margin: f32) -> Point {
        let (u, v) = self.to_uv(p);
        let reach = u.abs() + v.abs();
        let limit = 1.0 - margin;
        if reach <= limit {
            p
        } else {
            self.uv_point(u * limit / reach, v * limit / reach)
        }
    }

    pub fn corners(self, margin: f32) -> [Point; 4] {
        let k = 1.0 - margin;
        [
            self.uv_point(-k, 0.0),
            self.uv_point(k, 0.0),
            self.uv_point(0.0, k),
            self.uv_point(0.0, -k),
        ]
    }

    pub fn random(self, rng: &mut Rng, margin: f32) -> Point {
        let k = 1.0 - margin;
        let u = rng.range(-k, k);
        let span = k - u.abs();
        self.uv_point(u, rng.range(-span, span))
    }

    pub fn lanes(self, margin: f32, count: usize) -> Vec<Point> {
        let k = 1.0 - margin;
        let count = count.max(2);
        (0..count)
            .flat_map(|i| {
                let v = -k + 2.0 * k * (i as f32 + 0.5) / count as f32;
                let reach = (k - v.abs()).max(0.0);
                let ends = [self.uv_point(-reach, v), self.uv_point(reach, v)];
                if i % 2 == 0 { ends } else { [ends[1], ends[0]] }
            })
            .collect()
    }
}

pub fn ray(origin: Point, through: Point, distance: f32) -> Point {
    let dx = through.x - origin.x;
    let dy = through.y - origin.y;
    let len = dx.hypot(dy);
    if len == 0.0 {
        return Point::new(origin.x + distance, origin.y);
    }
    Point::new(origin.x + dx / len * distance, origin.y + dy / len * distance)
}

pub fn angle(from: Point, to: Point) -> f32 {
    (to.y - from.y).atan2(to.x - from.x)
}

pub fn polar(origin: Point, angle: f32, distance: f32) -> Point {
    Point::new(
        origin.x + angle.cos() * distance,
        origin.y + angle.sin() * distance,
    )
}

pub fn angle_gap(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(std::f32::consts::TAU);
    d.min(std::f32::consts::TAU - d)
}

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.unit()
    }

    /// A value in `lo..=hi` (inclusive). `hi <= lo` returns `lo`.
    pub fn range_u64(&mut self, lo: u64, hi: u64) -> u64 {
        if hi <= lo {
            return lo;
        }
        lo + self.next_u64() % (hi - lo + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diamond_matches_iso_corners() {
        let g = Geometry::new(608.0, 608.0).unwrap();
        let [left, right, top, bottom] = g.corners(0.0);
        assert_eq!(left, Point::new(0.0, 0.0));
        assert_eq!(right, Point::new(608.0, 0.0));
        assert_eq!(top, Point::new(304.0, 304.0));
        assert_eq!(bottom, Point::new(304.0, -304.0));
        assert!(g.contains(Point::new(304.0, -200.0), 0.0));
        assert!(!g.contains(Point::new(20.0, 200.0), 0.0));
    }

    #[test]
    fn random_points_stay_inside() {
        let g = Geometry::new(400.0, 300.0).unwrap();
        let mut rng = Rng::new(7);
        for _ in 0..2000 {
            assert!(g.contains(g.random(&mut rng, 0.1), 0.099));
        }
    }

    #[test]
    fn clamp_pulls_points_inside() {
        let g = Geometry::new(200.0, 200.0).unwrap();
        let p = g.clamp(Point::new(-50.0, 300.0), 0.1);
        assert!(g.contains(p, 0.099));
    }

    #[test]
    fn lanes_alternate_and_stay_inside() {
        let g = Geometry::new(600.0, 600.0).unwrap();
        let lanes = g.lanes(0.1, 5);
        assert_eq!(lanes.len(), 10);
        assert!(lanes.iter().all(|p| g.contains(*p, 0.099)));
        assert!(lanes[0].x < lanes[1].x && lanes[2].x > lanes[3].x);
    }
}

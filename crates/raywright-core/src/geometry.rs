//! Screen geometry. Unless a field says otherwise, coordinates are physical screen pixels.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }

    pub fn offset(self, dx: f64, dy: f64) -> Point {
        Point::new(self.x + dx, self.y + dy)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Rect { x, y, width, height }
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    /// Half-open: the right and bottom edges are outside.
    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.right() && p.y >= self.y && p.y < self.bottom()
    }

    pub fn contains_rect(&self, other: &Rect) -> bool {
        other.x >= self.x && other.y >= self.y && other.right() <= self.right() && other.bottom() <= self.bottom()
    }

    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let r = self.right().min(other.right());
        let b = self.bottom().min(other.bottom());
        (r > x && b > y).then(|| Rect::new(x, y, r - x, b - y))
    }

    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(x, y, self.right().max(other.right()) - x, self.bottom().max(other.bottom()) - y)
    }

    pub fn translate(&self, dx: f64, dy: f64) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.width, self.height)
    }

    pub fn scale(&self, s: f64) -> Rect {
        Rect::new(self.x * s, self.y * s, self.width * s, self.height * s)
    }

    /// Whole pixels covering this rect: `(x, y, w, h)`.
    pub fn to_pixels(&self) -> (i64, i64, u32, u32) {
        let x0 = self.x.floor();
        let y0 = self.y.floor();
        let w = (self.right().ceil() - x0).max(0.0);
        let h = (self.bottom().ceil() - y0).max(0.0);
        (x0 as i64, y0 as i64, w as u32, h as u32)
    }
}

impl From<raywright_protocol::Rect> for Rect {
    fn from(r: raywright_protocol::Rect) -> Self {
        Rect::new(r.x, r.y, r.w, r.h)
    }
}

impl From<Rect> for raywright_protocol::Rect {
    fn from(r: Rect) -> Self {
        raywright_protocol::Rect::new(r.x, r.y, r.width, r.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersect_and_contains() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersect(&b), Some(Rect::new(5.0, 5.0, 5.0, 5.0)));
        assert_eq!(a.intersect(&Rect::new(10.0, 0.0, 1.0, 1.0)), None);
        assert!(a.contains(Point::new(0.0, 9.9)));
        assert!(!a.contains(Point::new(10.0, 5.0)));
        assert!(a.contains_rect(&Rect::new(1.0, 1.0, 9.0, 9.0)));
    }

    #[test]
    fn to_pixels_covers_fractions() {
        assert_eq!(Rect::new(1.5, 2.2, 3.0, 1.0).to_pixels(), (1, 2, 4, 2));
    }
}

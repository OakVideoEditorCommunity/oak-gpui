//! Pure HSV color math for the color picker (no gpui dependency).

/// A color in the HSV model: `h` in degrees `0..360`, `s` and `v` in `0..1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HsvColor {
	/// Hue in degrees, `0..360` (wraps).
	pub h: f32,
	/// Saturation, `0..1`.
	pub s: f32,
	/// Value (brightness), `0..1`.
	pub v: f32,
}

impl HsvColor {
	/// Create a color, clamping components into range.
	pub fn new(h: f32, s: f32, v: f32) -> Self {
		Self {
			h: h.rem_euclid(360.0),
			s: s.clamp(0.0, 1.0),
			v: v.clamp(0.0, 1.0),
		}
	}

	/// Convert from linear RGB components in `0..1`.
	pub fn from_rgb(r: f32, g: f32, b: f32) -> Self {
		let max = r.max(g).max(b);
		let min = r.min(g).min(b);
		let delta = max - min;
		let h = if delta == 0.0 {
			0.0
		} else if max == r {
			60.0 * (((g - b) / delta).rem_euclid(6.0))
		} else if max == g {
			60.0 * ((b - r) / delta + 2.0)
		} else {
			60.0 * ((r - g) / delta + 4.0)
		};
		let s = if max == 0.0 { 0.0 } else { delta / max };
		Self::new(h, s, max)
	}

	/// Convert to linear RGB components in `0..1`.
	pub fn to_rgb(self) -> (f32, f32, f32) {
		let h = (self.h.rem_euclid(360.0)) / 60.0;
		let c = self.v * self.s;
		let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
		let m = self.v - c;
		let (r, g, b) = match h as u32 {
			0 => (c, x, 0.0),
			1 => (x, c, 0.0),
			2 => (0.0, c, x),
			3 => (0.0, x, c),
			4 => (x, 0.0, c),
			_ => (c, 0.0, x),
		};
		(r + m, g + m, b + m)
	}

	/// Convert to RGBA components in `0..255` (alpha given separately).
	pub fn to_rgba_u8(self, alpha: u8) -> [u8; 4] {
		let (r, g, b) = self.to_rgb();
		[
			(r * 255.0).round().clamp(0.0, 255.0) as u8,
			(g * 255.0).round().clamp(0.0, 255.0) as u8,
			(b * 255.0).round().clamp(0.0, 255.0) as u8,
			alpha,
		]
	}

	/// The fully-saturated hue at maximum value: `hsv(h, 1, 1)`.
	pub fn hue_swatch(h: f32) -> Self {
		Self::new(h, 1.0, 1.0)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn approx(a: f32, b: f32) -> bool {
		(a - b).abs() < 0.001
	}

	#[test]
	fn primary_colors_round_trip() {
		// Red.
		assert!(approx(HsvColor::from_rgb(1.0, 0.0, 0.0).h, 0.0));
		// Green.
		assert!(approx(HsvColor::from_rgb(0.0, 1.0, 0.0).h, 120.0));
		// Blue.
		assert!(approx(HsvColor::from_rgb(0.0, 0.0, 1.0).h, 240.0));
		// Yellow.
		assert!(approx(HsvColor::from_rgb(1.0, 1.0, 0.0).h, 60.0));
	}

	#[test]
	fn rgb_to_hsv_to_rgb_round_trip() {
		for (r, g, b) in [
			(0.2, 0.4, 0.6),
			(1.0, 0.5, 0.25),
			(0.9, 0.9, 0.9),
			(0.1, 0.1, 0.1),
		] {
			let hsv = HsvColor::from_rgb(r, g, b);
			let (r2, g2, b2) = hsv.to_rgb();
			assert!(approx(r, r2) && approx(g, g2) && approx(b, b2), "{hsv:?}");
		}
	}

	#[test]
	fn saturation_and_value_extremes() {
		// Gray has zero saturation.
		let gray = HsvColor::from_rgb(0.5, 0.5, 0.5);
		assert!(approx(gray.s, 0.0));
		assert!(approx(gray.v, 0.5));
		// Black has zero value.
		let black = HsvColor::from_rgb(0.0, 0.0, 0.0);
		assert!(approx(black.v, 0.0));
		// White is full value, zero saturation.
		let white = HsvColor::from_rgb(1.0, 1.0, 1.0);
		assert!(approx(white.v, 1.0) && approx(white.s, 0.0));
	}

	#[test]
	fn to_rgba_u8_clamps() {
		let red = HsvColor::new(0.0, 1.0, 1.0);
		assert_eq!(red.to_rgba_u8(255), [255, 0, 0, 255]);
		let black = HsvColor::new(120.0, 1.0, 0.0);
		assert_eq!(black.to_rgba_u8(128), [0, 0, 0, 128]);
	}

	#[test]
	fn hue_wraps() {
		assert!(approx(HsvColor::new(360.0, 1.0, 1.0).h, 0.0));
		assert!(approx(HsvColor::new(-30.0, 1.0, 1.0).h, 330.0));
		// Hue 360 == hue 0: same color.
		let (r1, g1, b1) = HsvColor::new(0.0, 1.0, 1.0).to_rgb();
		let (r2, g2, b2) = HsvColor::new(360.0, 1.0, 1.0).to_rgb();
		assert!((r1 - r2).abs() < 0.001 && (g1 - g2).abs() < 0.001 && (b1 - b2).abs() < 0.001);
	}
}

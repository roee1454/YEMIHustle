//! Fixed-point numbers: 64-bit signed, 16 fractional bits.
//!
//! Multiplication floors and division truncates toward zero, matching what the original game's
//! native library was measured to do (see notes/04). Floats only appear when data is loaded
//! ([`Fixed::from_f64`]) and when values are handed to the renderer ([`Fixed::to_f32`]).

use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

pub const FRAC_BITS: u32 = 16;
const ONE_RAW: i64 = 1 << FRAC_BITS;

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Fixed(pub i64);

impl Fixed {
    pub const ZERO: Fixed = Fixed(0);
    pub const ONE: Fixed = Fixed(ONE_RAW);

    pub const fn from_int(v: i64) -> Self {
        Fixed(v << FRAC_BITS)
    }

    /// Nearest representable value, ties away from zero. Only used when loading data.
    pub fn from_f64(v: f64) -> Self {
        Fixed((v * ONE_RAW as f64).round() as i64)
    }

    pub fn to_f32(self) -> f32 {
        self.0 as f32 / ONE_RAW as f32
    }

    /// Round to an integer, ties away from zero.
    pub fn round(self) -> i64 {
        let half = ONE_RAW / 2;
        if self.0 >= 0 {
            (self.0 + half) >> FRAC_BITS
        } else {
            -((-self.0 + half) >> FRAC_BITS)
        }
    }

    pub fn abs(self) -> Self {
        Fixed(self.0.abs())
    }

    pub fn signum(self) -> i64 {
        self.0.signum()
    }

    pub fn min(self, o: Self) -> Self {
        if self < o { self } else { o }
    }

    pub fn max(self, o: Self) -> Self {
        if self > o { self } else { o }
    }

    pub fn clamp(self, lo: Self, hi: Self) -> Self {
        self.max(lo).min(hi)
    }

    /// Full-precision square root (0 for non-positive input).
    pub fn sqrt(self) -> Self {
        if self.0 <= 0 {
            return Fixed::ZERO;
        }
        Fixed((((self.0 as u128) << FRAC_BITS).isqrt()) as i64)
    }
}

impl Add for Fixed {
    type Output = Fixed;
    fn add(self, o: Fixed) -> Fixed {
        Fixed(self.0 + o.0)
    }
}

impl Sub for Fixed {
    type Output = Fixed;
    fn sub(self, o: Fixed) -> Fixed {
        Fixed(self.0 - o.0)
    }
}

impl Neg for Fixed {
    type Output = Fixed;
    fn neg(self) -> Fixed {
        Fixed(-self.0)
    }
}

impl Mul for Fixed {
    type Output = Fixed;
    /// Floors (rounds toward minus infinity).
    fn mul(self, o: Fixed) -> Fixed {
        Fixed(((self.0 as i128 * o.0 as i128) >> FRAC_BITS) as i64)
    }
}

impl Div for Fixed {
    type Output = Fixed;
    /// Truncates toward zero. Division by zero yields zero instead of panicking, so a bad data
    /// value cannot crash a match.
    fn div(self, o: Fixed) -> Fixed {
        if o.0 == 0 {
            return Fixed::ZERO;
        }
        Fixed((((self.0 as i128) << FRAC_BITS) / o.0 as i128) as i64)
    }
}

impl AddAssign for Fixed {
    fn add_assign(&mut self, o: Fixed) {
        self.0 += o.0;
    }
}

impl SubAssign for Fixed {
    fn sub_assign(&mut self, o: Fixed) {
        self.0 -= o.0;
    }
}

/// Converts an authoring value from the data format.
pub fn fx(v: charfmt::Fx) -> Fixed {
    Fixed::from_f64(v.0)
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct FVec {
    pub x: Fixed,
    pub y: Fixed,
}

impl FVec {
    pub const ZERO: FVec = FVec { x: Fixed::ZERO, y: Fixed::ZERO };

    pub fn new(x: Fixed, y: Fixed) -> Self {
        FVec { x, y }
    }

    pub fn len(self) -> Fixed {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    /// Unit vector in the same direction (zero stays zero).
    pub fn normalized(self) -> FVec {
        let l = self.len();
        if l == Fixed::ZERO {
            return FVec::ZERO;
        }
        FVec::new(self.x / l, self.y / l)
    }

    pub fn scale(self, s: Fixed) -> FVec {
        FVec::new(self.x * s, self.y * s)
    }
}

impl Add for FVec {
    type Output = FVec;
    fn add(self, o: FVec) -> FVec {
        FVec::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for FVec {
    type Output = FVec;
    fn sub(self, o: FVec) -> FVec {
        FVec::new(self.x - o.x, self.y - o.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding_rules_match_the_original() {
        let unit = Fixed(1);
        let half = Fixed::from_f64(0.5);
        // multiply floors: -3 units * 0.5 = -1.5 units -> -2
        assert_eq!((Fixed(-3) * half).0, -2);
        // divide truncates: -1 unit / 2 -> 0
        assert_eq!((Fixed(-1) / Fixed::from_int(2)).0, 0);
        assert_eq!((unit * half).0, 0);
        // rounding ties away from zero
        assert_eq!(Fixed::from_f64(2.5).round(), 3);
        assert_eq!(Fixed::from_f64(-2.5).round(), -3);
        // 0.1 parses to the nearest unit, so 0.1 * 3 is slightly above 0.3 (as in the original)
        let tenth = Fixed::from_f64(0.1);
        assert_eq!(tenth.0, 6554);
    }

    #[test]
    fn sqrt_and_normalize() {
        assert_eq!(Fixed::from_int(25).sqrt(), Fixed::from_int(5));
        let v = FVec::new(Fixed::from_int(3), Fixed::from_int(4)).normalized();
        // division truncates, so 3/5 is one unit below the nearest value of 0.6
        assert_eq!(v.x, Fixed::from_int(3) / Fixed::from_int(5));
        assert_eq!(v.y, Fixed::from_int(4) / Fixed::from_int(5));
        assert_eq!(Fixed::ONE / Fixed::ZERO, Fixed::ZERO);
    }
}

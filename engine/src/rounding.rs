//! OpenDominion's custom rounding helpers (src/helpers.php @ round-50), replicated bit-for-bit.
//!
//! PHP's `round()` rounds half away from zero, which matches Rust's `f64::round()`.
//!   rfloor($x) = floor(round($x, 10))   // 10-decimal pre-round kills float noise
//!   rceil($x)  = ceil(round($x, 10))
//!   clamp($c, $min, $max) = max($min, min($max, $c))

/// PHP `round($x, $precision)` — half away from zero at `precision` decimals.
pub fn php_round(x: f64, precision: i32) -> f64 {
    let f = 10f64.powi(precision);
    (x * f).round() / f
}

/// `rfloor($x)` = floor(round($x, 10)).
pub fn rfloor(x: f64) -> i64 {
    php_round(x, 10).floor() as i64
}

/// `rceil($x)` = ceil(round($x, 10)).
pub fn rceil(x: f64) -> i64 {
    php_round(x, 10).ceil() as i64
}

/// PHP `round($x)` to the nearest integer (half away from zero).
pub fn round_int(x: f64) -> i64 {
    x.round() as i64
}

/// `clamp($current, $min, $max)` = max($min, min($max, $current)).
pub fn clamp(current: f64, min: f64, max: f64) -> f64 {
    min.max(max.min(current))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfloor_matches_php() {
        assert_eq!(rfloor(975.0), 975);
        assert_eq!(rfloor(975.99), 975);
        assert_eq!(rfloor(116.0), 116);
        // explore draftee cost: floor(350/150)+3 path uses rfloor(350.0/150.0)=2
        assert_eq!(rfloor(350.0 / 150.0), 2);
    }

    #[test]
    fn rceil_matches_php() {
        assert_eq!(rceil(116.0), 116);
        assert_eq!(rceil(116.01), 117);
        assert_eq!(rceil(0.0), 0);
    }

    #[test]
    fn round_int_is_half_away_from_zero() {
        assert_eq!(round_int(2.5), 3);
        assert_eq!(round_int(3.4), 3);
        // construct platinum raw at start: 850 + 1.25*100 = 975
        assert_eq!(round_int(975.0), 975);
        // construct lumber raw at start: 87.5 + 0.285*100 = 116.0
        assert_eq!(round_int(87.5 + 0.285 * 100.0), 116);
    }

    #[test]
    fn clamp_matches_php() {
        assert_eq!(clamp(0.6875, 0.35, 0.5), 0.5);
        assert_eq!(clamp(0.2, 0.35, 0.5), 0.35);
        assert_eq!(clamp(0.4, 0.35, 0.5), 0.4);
    }
}

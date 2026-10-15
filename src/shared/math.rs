//! Scalar helpers shared by the effect and fog formulas.

/// Interpolates from `a` to `b`, clamping `t` to `[0, 1]` first.
#[inline]
pub fn lerp_clamped(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// Where `value` sits between `a` and `b`, clamped to `[0, 1]`. Returns 0 when `a == b`.
#[inline]
pub fn inverse_lerp_clamped(a: f32, b: f32, value: f32) -> f32 {
    if a == b {
        0.0
    } else {
        ((value - a) / (b - a)).clamp(0.0, 1.0)
    }
}

/// Moves `current` towards `target` by at most `max_delta`, never overshooting.
#[inline]
pub fn move_towards(current: f32, target: f32, max_delta: f32) -> f32 {
    let delta = target - current;
    if delta.abs() <= max_delta {
        target
    } else {
        current + delta.signum() * max_delta
    }
}

/// Cubic Hermite smoothstep of `value` clamped to `[0, 1]`.
#[inline]
pub fn smoothstep01(value: f32) -> f32 {
    let value = value.clamp(0.0, 1.0);
    value * value * (3.0 - 2.0 * value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lerp_clamps_its_parameter() {
        assert_eq!(lerp_clamped(2.0, 4.0, 0.5), 3.0);
        assert_eq!(lerp_clamped(2.0, 4.0, -1.0), 2.0);
        assert_eq!(lerp_clamped(2.0, 4.0, 3.0), 4.0);
        assert_eq!(lerp_clamped(1.0, 0.0, 0.25), 0.75);
    }

    #[test]
    fn inverse_lerp_is_clamped_and_handles_empty_ranges() {
        assert_eq!(inverse_lerp_clamped(115.0, 180.0, 147.5), 0.5);
        assert_eq!(inverse_lerp_clamped(115.0, 180.0, 0.0), 0.0);
        assert_eq!(inverse_lerp_clamped(115.0, 180.0, 400.0), 1.0);
        assert_eq!(inverse_lerp_clamped(3.0, 3.0, 9.0), 0.0);
    }

    #[test]
    fn move_towards_never_overshoots() {
        assert_eq!(move_towards(0.0, 0.25, 1.0), 0.25);
        assert_eq!(move_towards(1.0, 0.0, 0.1), 0.9);
        assert_eq!(move_towards(-1.0, 1.0, 0.5), -0.5);
        assert_eq!(move_towards(0.5, 0.5, 0.0), 0.5);
    }

    #[test]
    fn smoothstep_has_flat_ends() {
        assert_eq!(smoothstep01(-1.0), 0.0);
        assert_eq!(smoothstep01(0.5), 0.5);
        assert_eq!(smoothstep01(2.0), 1.0);
    }
}

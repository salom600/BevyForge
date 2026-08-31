//! Minimal rotation math shared by the editor (offline scene edits) and the
//! runtime (consistency tests against Bevy's `Quat`).
//!
//! Conventions mirror `bevy::math::Quat` / glam:
//! * Hamilton products (`a * b` applies `b` first, then `a`);
//! * `from_euler_xyz` builds the **intrinsic XYZ** composition `qx * qy * qz`;
//! * `to_euler_xyz` is its exact reciprocal (angles in radians).
//!
//! `forge_runtime` unit-tests these against Bevy itself, so a convention
//! mismatch is caught at build time instead of corrupting scene rotations.

/// A plain quaternion (x, y, z, w).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Self = Self { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };

    pub fn from_axis_angle(axis: [f32; 3], angle_rad: f32) -> Self {
        let len = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
        if len < 1e-9 {
            return Self::IDENTITY;
        }
        let s = (angle_rad * 0.5).sin() / len;
        Self {
            x: axis[0] * s,
            y: axis[1] * s,
            z: axis[2] * s,
            w: (angle_rad * 0.5).cos(),
        }
    }

    /// Intrinsic XYZ euler (radians) → quaternion, matching glam's
    /// `Quat::from_euler(EulerRot::XYZ, x, y, z)` (= `qx * qy * qz`).
    pub fn from_euler_xyz(x: f32, y: f32, z: f32) -> Self {
        let (sx, cx) = (x * 0.5).sin_cos();
        let (sy, cy) = (y * 0.5).sin_cos();
        let (sz, cz) = (z * 0.5).sin_cos();
        // q = qx * qy * qz expanded (Hamilton product, column-vector convention).
        Self {
            x: sx * cy * cz + cx * sy * sz,
            y: cx * sy * cz - sx * cy * sz,
            z: cx * cy * sz + sx * sy * cz,
            w: cx * cy * cz - sx * sy * sz,
        }
    }

    /// Quaternion → intrinsic XYZ euler (radians); reciprocal of
    /// [`Self::from_euler_xyz`] (equivalent to glam's
    /// `Quat::to_euler(EulerRot::XYZ)`).
    ///
    /// Extracted from `R = Rx(x) * Ry(y) * Rz(z)` using the standard
    /// quaternion-matrix entries:
    /// `R02 = 2(xz+wy) = sin y`, `R12 = 2(yz−wx) = −sin x·cos y`,
    /// `R22 = 1−2(x²+y²) = cos x·cos y`, `R01 = 2(xy−wz) = −cos y·sin z`,
    /// `R00 = 1−2(y²+z²) = cos y·cos z`.
    pub fn to_euler_xyz(self) -> [f32; 3] {
        let Self { x, y, z, w } = self;
        let sin_y = 2.0 * (x * z + w * y);
        if sin_y.abs() >= 0.999_999 {
            // Gimbal lock (pitch ±90°): only x±z is defined. Recover the free
            // combination from R10/R11 and fold it into x (z = 0) — a valid
            // decomposition of the same rotation.
            let r10 = 2.0 * (x * y + w * z);
            let r11 = 1.0 - 2.0 * (x * x + z * z);
            let combo = r10.atan2(r11);
            if sin_y > 0.0 {
                return [combo, std::f32::consts::FRAC_PI_2, 0.0];
            }
            return [-combo, -std::f32::consts::FRAC_PI_2, 0.0];
        }
        let y_ang = sin_y.asin();
        let x_ang = (-2.0 * (y * z - w * x)).atan2(1.0 - 2.0 * (x * x + y * y));
        let z_ang = (-2.0 * (x * y - w * z)).atan2(1.0 - 2.0 * (y * y + z * z));
        [x_ang, y_ang, z_ang]
    }

    /// Hamilton product `self * rhs` (apply `rhs` first).
    pub fn mul(self, rhs: Self) -> Self {
        Self {
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_round_trip() {
        for x in [-1.2, 0.0, 0.7, 2.4] {
            for y in [-2.8, 0.0, 1.1] {
                for z in [-0.4, 0.0, 3.0] {
                    let q = Quat::from_euler_xyz(x, y, z);
                    let [x2, y2, z2] = q.to_euler_xyz();
                    let q2 = Quat::from_euler_xyz(x2, y2, z2);
                    // Same rotation up to double cover (q ≈ -q).
                    let dot = (q.x * q2.x + q.y * q2.y + q.z * q2.z + q.w * q2.w).abs();
                    assert!(dot > 0.999_999, "({x},{y},{z}) -> ({x2},{y2},{z2}) dot {dot}");
                }
            }
        }
    }
}

//! Guards the shared euler↔quat math in `forge_ipc::math` against drift from
//! Bevy's own `Quat` conventions. The editor's offline scene applier relies on
//! these being compatible: euler values it writes into scene documents are
//! later interpreted by Bevy's `Quat::from_euler(EulerRot::XYZ, ..)`.

use bevy::math::{EulerRot, Quat as BevyQuat};

use forge_ipc::math::Quat;

fn bevy_to_euler(q: BevyQuat) -> [f32; 3] {
    let (x, y, z) = q.to_euler(EulerRot::XYZ);
    [x, y, z]
}

#[test]
fn ipc_math_matches_bevy() {
    let samples: &[([f32; 3], &str)] = &[
        ([0.0, 0.0, 0.0], "identity"),
        ([-90.0f32.to_radians(), 0.0, 0.0], "default sun pitch"),
        ([35.0f32.to_radians(), -20.0f32.to_radians(), 10.0f32.to_radians()], "generic deg"),
        ([2.4, -1.1, 0.3], "generic radians"),
        ([179.9f32.to_radians(), 0.0, 0.0], "near fold"),
        ([45.0f32.to_radians(), 89.0f32.to_radians(), 45.0f32.to_radians()], "near gimbal"),
    ];
    for ([x, y, z], name) in samples {
        let bevy_q = BevyQuat::from_euler(EulerRot::XYZ, *x, *y, *z);
        let ipc_q = Quat::from_euler_xyz(*x, *y, *z);
        let dot = (bevy_q.x * ipc_q.x + bevy_q.y * ipc_q.y + bevy_q.z * ipc_q.z + bevy_q.w * ipc_q.w).abs();
        assert!(dot > 0.999_999, "{name}: from_euler mismatch (dot {dot}) for ({x},{y},{z})");

        // The editor's to_euler must describe the SAME rotation when Bevy
        // interprets it (alternative-but-valid decompositions are fine).
        let ipc_e = ipc_q.to_euler_xyz();
        let bq2 = BevyQuat::from_euler(EulerRot::XYZ, ipc_e[0], ipc_e[1], ipc_e[2]);
        let dot2 = (bevy_q.x * bq2.x + bevy_q.y * bq2.y + bevy_q.z * bq2.z + bevy_q.w * bq2.w).abs();
        assert!(dot2 > 0.999_999, "{name}: to_euler decomposition differs (dot {dot2})");
    }
}

#[test]
fn rotate_world_axis_matches_bevy() {
    // The offline RotateEntityWorld semantics: q' = q_axis * q, then back to
    // euler. The runtime does exactly this with Bevy types; assert parity.
    for deg in [-40.0_f32, 12.0, 130.0] {
        let rad = deg.to_radians();
        let start = BevyQuat::from_euler(EulerRot::XYZ, 0.3, 0.7, -1.0);
        let rotated = BevyQuat::from_axis_angle(bevy::math::Vec3::Y, rad) * start;

        let start_ipc = Quat::from_euler_xyz(0.3, 0.7, -1.0);
        let rotated_ipc = Quat::from_axis_angle([0.0, 1.0, 0.0], rad).mul(start_ipc);
        let ipc_euler = rotated_ipc.to_euler_xyz();

        let back = BevyQuat::from_euler(EulerRot::XYZ, ipc_euler[0], ipc_euler[1], ipc_euler[2]);
        let dot = (rotated.x * back.x + rotated.y * back.y + rotated.z * back.z + rotated.w * back.w).abs();
        assert!(dot > 0.999_999, "rotate {deg}°: offline euler ({ipc_euler:?}) vs bevy euler dot {dot}");
    }
}

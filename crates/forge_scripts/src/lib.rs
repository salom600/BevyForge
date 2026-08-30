//! # forge_scripts — the BevyForge scripting crate
//!
//! This crate is what the in-editor **Script Editor** opens and edits. It holds
//! every user-facing gameplay component plus the systems that drive them during
//! **Play Mode**. The runtime registers these types in Bevy's `TypeRegistry`,
//! which is what makes them serialisable to scenes and inspectable in the editor.
//!
//! Editing a file here, then running `Check` in the editor's Rust Compiler
//! panel, compiles this crate — the exact workflow a Bevy programmer uses daily.

use bevy::math::Vec3;
use bevy::prelude::*;

// ---------------------------------------------------------------------------
// Gameplay data components (values mirror the BevyForge design blueprint)
// ---------------------------------------------------------------------------

/// Player-tuning bundle attached to the Player prefab.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
pub struct Player {
    /// Ground movement speed in metres per second.
    pub speed: f32,
    /// Initial vertical impulse when jumping.
    pub jump_force: f32,
    /// Multiplier applied while sprint is held.
    pub sprint_multiplier: f32,
}

impl Default for Player {
    fn default() -> Self {
        Self { speed: 12.0, jump_force: 25.0, sprint_multiplier: 1.5 }
    }
}

/// Kinematic body parameters consumed by play-mode movement.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
pub struct CharacterController {
    pub height: f32,
    pub radius: f32,
    pub step_offset: f32,
    /// Maximum walkable slope, in degrees.
    pub slope_limit: f32,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self { height: 2.0, radius: 0.35, step_offset: 0.5, slope_limit: 45.0 }
    }
}

/// Simple destructible-style health pool.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Default for Health {
    fn default() -> Self {
        Self { current: 100.0, max: 100.0 }
    }
}

/// Carrying capacity.
#[derive(Component, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Component, Default)]
pub struct Inventory {
    pub slots: u32,
}

impl Default for Inventory {
    fn default() -> Self {
        Self { slots: 32 }
    }
}

// ---------------------------------------------------------------------------
// Behaviour components — these actually animate entities in Play Mode
// ---------------------------------------------------------------------------

/// Constant angular velocity, radians per second on each axis.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
pub struct Rotator {
    pub speed: Vec3,
}

impl Default for Rotator {
    fn default() -> Self {
        Self { speed: Vec3::new(0.0, 1.0, 0.0) }
    }
}

/// Circular motion around a centre point.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
pub struct Orbiter {
    pub center: Vec3,
    pub radius: f32,
    /// Radians per second.
    pub speed: f32,
    /// Internal angle accumulator (radians).
    pub angle: f32,
}

impl Default for Orbiter {
    fn default() -> Self {
        Self { center: Vec3::ZERO, radius: 3.0, speed: 1.0, angle: 0.0 }
    }
}

/// Straight-line motion with optional ping-pong reversal.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
pub struct LinearMover {
    pub velocity: Vec3,
    /// Reverse direction after `travel_range` metres from `origin`.
    pub ping_pong: bool,
    /// Metres of travel before reversing (when `ping_pong`).
    pub travel_range: f32,
    /// Auto-captured spawn position.
    pub origin: Vec3,
}

impl Default for LinearMover {
    fn default() -> Self {
        Self {
            velocity: Vec3::new(1.0, 0.0, 0.0),
            ping_pong: true,
            travel_range: 5.0,
            origin: Vec3::ZERO,
        }
    }
}

/// Sinusoidal oscillation from the captured origin along an offset vector.
#[derive(Component, Reflect, Debug, Clone, PartialEq)]
#[reflect(Component, Default)]
pub struct PingPongMover {
    /// Displacement vector at the oscillation peak.
    pub offset: Vec3,
    /// Seconds for a full there-and-back cycle.
    pub period: f32,
    /// Auto-captured spawn position.
    pub base: Vec3,
}

impl Default for PingPongMover {
    fn default() -> Self {
        Self { offset: Vec3::new(0.0, 2.0, 0.0), period: 2.0, base: Vec3::ZERO }
    }
}

// ---------------------------------------------------------------------------
// Play-mode systems
// ---------------------------------------------------------------------------

/// Spin every `Rotator` around its local axes.
pub fn rotate_system(time: Res<Time>, mut query: Query<(&Rotator, &mut Transform)>) {
    for (rotator, mut transform) in &mut query {
        transform.rotate_x(rotator.speed.x * time.delta_secs());
        transform.rotate_y(rotator.speed.y * time.delta_secs());
        transform.rotate_z(rotator.speed.z * time.delta_secs());
    }
}

/// Drive every `Orbiter` along its circle.
pub fn orbit_system(time: Res<Time>, mut query: Query<(&mut Orbiter, &mut Transform)>) {
    for (mut orbiter, mut transform) in &mut query {
        orbiter.angle += orbiter.speed * time.delta_secs();
        let (sin, cos) = orbiter.angle.sin_cos();
        transform.translation = orbiter.center + Vec3::new(cos * orbiter.radius, 0.0, sin * orbiter.radius);
    }
}

/// Straight motion with optional ping-pong at `travel_range` from origin.
pub fn linear_move_system(time: Res<Time>, mut query: Query<(&mut LinearMover, &mut Transform)>) {
    for (mut mover, mut transform) in &mut query {
        if mover.origin == Vec3::ZERO && transform.translation != Vec3::ZERO && mover.velocity != Vec3::ZERO {
            // First tick in play mode: remember where we started.
            mover.origin = transform.translation;
        }
        let mut next = transform.translation + mover.velocity * time.delta_secs();
        if mover.ping_pong && mover.travel_range > 0.0 {
            let travelled = (next - mover.origin).dot(mover.velocity.normalize_or_zero());
            if travelled >= mover.travel_range {
                next = mover.origin + mover.velocity.normalize_or_zero() * mover.travel_range;
                mover.velocity = -mover.velocity;
            } else if travelled <= -mover.travel_range {
                next = mover.origin - mover.velocity.normalize_or_zero() * mover.travel_range;
                mover.velocity = -mover.velocity;
            }
        }
        transform.translation = next;
    }
}

/// Smooth oscillation along `offset`.
pub fn ping_pong_system(time: Res<Time>, mut query: Query<(&mut PingPongMover, &mut Transform)>) {
    for (mut mover, mut transform) in &mut query {
        if mover.base == Vec3::ZERO && transform.translation != Vec3::ZERO {
            mover.base = transform.translation;
        }
        let phase = (time.elapsed_secs() * std::f32::consts::TAU / mover.period.max(0.001)).sin();
        transform.translation = mover.base + mover.offset * (0.5 + 0.5 * phase);
    }
}

/// Player patrol: walks forward, reverses on a 10 m runway, sprints on the
/// return leg — exercises `speed` and `sprint_multiplier` headlessly.
pub fn player_patrol_system(time: Res<Time>, mut query: Query<(&Player, &mut Transform)>) {
    for (player, mut transform) in &mut query {
        let direction = if transform.translation.x < -5.0 { 1.0 } else if transform.translation.x > 5.0 { -1.0 } else { 0.0 };
        let speed = player.speed
            * if direction < 0.0 { player.sprint_multiplier } else { 1.0 };
        if direction == 0.0 && transform.translation.x < 0.0 {
            transform.translation.x += speed * time.delta_secs();
        } else if direction == 0.0 && transform.translation.x > 0.0 {
            transform.translation.x -= speed * time.delta_secs();
        } else {
            transform.translation.x += direction * speed * time.delta_secs();
        }
    }
}

/// Convenience plugin registering all script types with the app `TypeRegistry`.
pub struct ForgeScriptsPlugin;

impl Plugin for ForgeScriptsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Player>()
            .register_type::<CharacterController>()
            .register_type::<Health>()
            .register_type::<Inventory>()
            .register_type::<Rotator>()
            .register_type::<Orbiter>()
            .register_type::<LinearMover>()
            .register_type::<PingPongMover>();
    }
}

/// Boxed script component used by scene loading to insert the right type.
#[derive(Debug, Clone)]
pub enum AnyScript {
    Rotator(Rotator),
    Orbiter(Orbiter),
    LinearMover(LinearMover),
    PingPongMover(PingPongMover),
    Player(Player),
    CharacterController(CharacterController),
    Health(Health),
    Inventory(Inventory),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_blueprint_defaults() {
        // Values must match the BevyForge design file exactly.
        let player = Player::default();
        assert_eq!((player.speed, player.jump_force, player.sprint_multiplier), (12.0, 25.0, 1.5));
        let cc = CharacterController::default();
        assert_eq!((cc.height, cc.radius, cc.step_offset, cc.slope_limit), (2.0, 0.35, 0.5, 45.0));
        let health = Health::default();
        assert_eq!((health.current, health.max), (100.0, 100.0));
        assert_eq!(Inventory::default().slots, 32);
    }
}

//! Editor-authored keyframe animation: per-entity transform tracks with
//! linear interpolation, driven by name-keyed stores (stable across play-mode
//! rollback and scene reloads).

use bevy::math::{EulerRot, Quat, Vec3};
use bevy::prelude::*;

use forge_ipc::{AnimEntityTracks, AnimTrack};

use crate::state::IpcChannels;

/// Sorted keyframe list for one track: (time seconds, value xyz).
pub type KeyframeList = Vec<(f32, [f32; 3])>;

/// All tracks authored for one entity (keyed by entity name).
#[derive(Debug, Clone, Default)]
pub struct EntityTracks {
    pub tracks: Vec<(AnimTrack, KeyframeList)>,
}

/// Name-keyed animation store.
#[derive(Debug, Default, Resource)]
pub struct AnimationStore {
    pub by_name: std::collections::BTreeMap<String, EntityTracks>,
}

impl AnimationStore {
    pub fn track_mut(&mut self, name: &str, track: AnimTrack) -> &mut KeyframeList {
        let entry = self.by_name.entry(name.to_string()).or_default();
        if !entry.tracks.iter().any(|(t, _)| *t == track) {
            entry.tracks.push((track, Vec::new()));
        }
        let idx = entry
            .tracks
            .iter()
            .position(|(t, _)| *t == track)
            .expect("track just inserted");
        &mut entry.tracks[idx].1
    }

    pub fn clear_entity(&mut self, name: &str) {
        self.by_name.remove(name);
    }
}

/// Global playback state.
#[derive(Debug, Clone, Resource)]
pub struct AnimPlayback {
    pub time: f32,
    pub duration: f32,
    pub playing: bool,
    pub looped: bool,
}

impl Default for AnimPlayback {
    fn default() -> Self {
        Self { time: 0.0, duration: 30.0, playing: false, looped: true }
    }
}

/// Linear interpolation over a sorted keyframe list with looping.
pub fn sample_track(keys: &KeyframeList, t: f32, duration: f32, looped: bool) -> Option<[f32; 3]> {
    if keys.is_empty() {
        return None;
    }
    if keys.len() == 1 {
        return Some(keys[0].1);
    }
    let mut time = t;
    if looped && duration > 0.0 {
        time = time.rem_euclid(duration);
    }
    let first = keys.first()?;
    let last = keys.last()?;
    if time <= first.0 {
        return Some(first.1);
    }
    if time >= last.0 {
        if looped {
            // Wrap segment: last → (first + duration).
            let seg_len = (first.0 + duration) - last.0;
            if seg_len > f32::EPSILON {
                let a = (time - last.0) / seg_len;
                let value = [0.0; 3];
                let mut out = value;
                for i in 0..3 {
                    out[i] = last.1[i] + (first.1[i] - last.1[i]) * a;
                }
                return Some(out);
            }
        }
        return Some(last.1);
    }
    // Binary search the bracketing segment.
    let mut lo = 0usize;
    let mut hi = keys.len() - 1;
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if keys[mid].0 <= time {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let (t0, v0) = keys[lo];
    let (t1, v1) = keys[hi];
    let a = if t1 - t0 > f32::EPSILON { (time - t0) / (t1 - t0) } else { 0.0 };
    let mut out = [0.0f32; 3];
    for i in 0..3 {
        out[i] = v0[i] + (v1[i] - v0[i]) * a;
    }
    Some(out)
}

/// Advance playback time.
pub fn advance_time(mut playback: ResMut<AnimPlayback>, time: Res<Time>) {
    if !playback.playing {
        return;
    }
    playback.time += time.delta_secs();
    if playback.time > playback.duration {
        if playback.looped {
            playback.time %= playback.duration.max(0.001);
        } else {
            playback.time = playback.duration;
            playback.playing = false;
        }
    }
}

/// Apply animated transforms (runs before the gameplay systems).
pub fn apply_animation(
    playback: Res<AnimPlayback>,
    store: Res<AnimationStore>,
    mut previous: Local<f32>,
    mut query: Query<(&Name, &mut Transform)>,
    mut flags: ResMut<crate::state::RuntimeFlags>,
) {
    let time_changed = (playback.time - *previous).abs() > f32::EPSILON;
    if time_changed {
        *previous = playback.time;
    }
    if !playback.playing && !time_changed {
        return;
    }
    for (name, mut transform) in &mut query {
        let Some(tracks) = store.by_name.get(name.as_str()) else { continue };
        for (track, keys) in &tracks.tracks {
            let Some(value) = sample_track(keys, playback.time, playback.duration, playback.looped)
            else {
                continue;
            };
            let v = Vec3::new(value[0], value[1], value[2]);
            match track {
                AnimTrack::Translation => transform.translation = v,
                AnimTrack::Rotation => {
                    transform.rotation = Quat::from_euler(
                        EulerRot::XYZ,
                        v.x.to_radians(),
                        v.y.to_radians(),
                        v.z.to_radians(),
                    )
                }
                AnimTrack::Scale => transform.scale = v,
            }
        }
    }
    flags.anim_dirty = playback.playing; // keep scrubber in sync while playing
}

/// Push animation state + tracks to the editor timeline.
pub fn push_anim_state(
    mut flags: ResMut<crate::state::RuntimeFlags>,
    playback: Res<AnimPlayback>,
    store: Res<AnimationStore>,
    names: Query<(Entity, &Name)>,
    channels: Res<IpcChannels>,
    mut throttle: Local<u32>,
) {
    *throttle += 1;
    let should_push = flags.anim_dirty || (*throttle % 30 == 0 && playback.playing);
    if !should_push {
        return;
    }
    flags.anim_dirty = false;
    let mut tracks: Vec<AnimEntityTracks> = Vec::new();
    for (entity, name) in &names {
        if let Some(entry) = store.by_name.get(name.as_str()) {
            tracks.push(AnimEntityTracks {
                entity: entity.to_bits(),
                name: name.as_str().to_string(),
                tracks: entry.tracks.clone(),
            });
        }
    }
    tracks.sort_by(|a, b| a.name.cmp(&b.name));
    let _ = channels.evt_tx.send(forge_ipc::RuntimeToEditor::AnimState {
        state: forge_ipc::AnimState {
            time: playback.time,
            duration: playback.duration,
            playing: playback.playing,
            looped: playback.looped,
        },
        tracks,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_middle_keyframes() {
        let keys = vec![(0.0, [0.0; 3]), (2.0, [10.0, 0.0, 0.0]), (4.0, [0.0, 0.0, 20.0])];
        assert_eq!(sample_track(&keys, 1.0, 4.0, false), Some([5.0, 0.0, 0.0]));
        assert_eq!(sample_track(&keys, 0.0, 4.0, false), Some([0.0; 3]));
        assert_eq!(sample_track(&keys, 4.0, 4.0, false), Some([0.0, 0.0, 20.0]));
        assert_eq!(sample_track(&keys, -1.0, 4.0, false), Some([0.0; 3]));
    }

    #[test]
    fn loops_past_duration() {
        let keys = vec![(0.0, [0.0; 3]), (2.0, [4.0, 0.0, 0.0])];
        // duration 4: t=3 → halfway back from (4,0,0) to (0,0,0)
        assert_eq!(sample_track(&keys, 3.0, 4.0, true), Some([2.0, 0.0, 0.0]));
    }
}

//! Offline scene document — the editor's own authoritative world while the
//! render engine is down.
//!
//! BevyForge's architecture puts the ECS `World` in `bevyforge-runtime`, but
//! that must never make the editor a hollow shell when the engine cannot
//! start (missing/quarantined exe, broken GPU driver, blocked port). This
//! module applies the same [`forge_ipc::EditorToRuntime`] commands the runtime
//! would apply — locally, on the shared [`forge_ipc::scene_doc::ForgeScene`]
//! document — so hierarchy, inspector, gizmos, undo/redo, save/open and
//! environment editing all genuinely work offline.
//!
//! When the engine later connects, the dirty document is shipped to the
//! runtime with `EditorToRuntime::LoadSceneDoc` and entity ids are re-assigned
//! there; the editor therefore clears its undo stack at that boundary.

use std::collections::HashSet;

use forge_ipc::math::Quat;
use forge_ipc::scene_doc::{
    ForgeScene, SceneAnimEntry, SceneCamera, SceneEntity, SceneEntityKind, SceneLight,
    SceneMaterial, SceneScript,
};
use forge_ipc::{
    AnimState, ComponentData, ComponentField, ComponentKind, EditorToRuntime,
    EntityKind, FieldRow, FieldValue, HierNode, LogLevel, MeshPrimitive, NodeIcon,
};

/// Offline ids start high so they are visually distinct in logs (and never
/// collide with `NO_ENTITY`).
const FIRST_ID: u64 = 1000;

/// Result of applying one command offline.
#[derive(Debug, Default)]
pub struct Feedback {
    /// Entity id created by a `SpawnEntity` (so the UI can select it).
    pub spawned: Option<u64>,
    /// User-facing notice (toast/console).
    pub notice: Option<(LogLevel, String)>,
    /// Completed gizmo gesture → exact undo/redo batches (mirrors the
    /// runtime's `GestureDone` flow).
    pub gesture_done: Option<(String, Vec<EditorToRuntime>, Vec<EditorToRuntime>)>,
    /// True when the command needs the real engine (play mode, picking,
    /// screenshots, …) and was *not* applied locally.
    pub needs_engine: bool,
}

/// In-progress gizmo gesture (pre-transform snapshot).
struct Gesture {
    id: u64,
    pre: ([f32; 3], [f32; 3], [f32; 3]),
}

/// The offline world.
pub struct OfflineScene {
    pub doc: ForgeScene,
    /// Offline entity id per `doc.entities` index.
    pub ids: Vec<u64>,
    /// Parent (offline id) per index — parenting by index keeps renames safe.
    pub parents: Vec<Option<u64>>,
    next_id: u64,
    names: HashSet<String>,
    scene_path: Option<String>,
    /// Edits exist that the engine has never seen.
    pub unsynced: bool,
    /// Hierarchy/inspector mirror needs a rebuild.
    pub mirror_dirty: bool,
    gesture: Option<Gesture>,
    pub anim: AnimState,
}

impl OfflineScene {
    // -- construction ----------------------------------------------------

    /// Seed from the project's main scene file (or a default rig).
    pub fn from_project(project: Option<&forge_editor_core::Project>) -> Self {
        let mut me = Self {
            doc: ForgeScene::default(),
            ids: Vec::new(),
            parents: Vec::new(),
            next_id: FIRST_ID,
            names: HashSet::new(),
            scene_path: None,
            unsynced: false,
            mirror_dirty: true,
            gesture: None,
            anim: AnimState::default(),
        };
        let main = project
            .map(|p| p.resolve_scene(""))
            .unwrap_or_else(|| std::path::PathBuf::from("assets/scenes/main.scn.ron"));
        if main.is_file() {
            if let Ok(text) = std::fs::read_to_string(&main) {
                if let Ok(doc) = ron::from_str::<ForgeScene>(&text) {
                    me.adopt(doc, Some(main.to_string_lossy().to_string()));
                } else {
                    me.push_default_rig();
                }
            }
        } else {
            me.push_default_rig();
        }
        me
    }

    /// Replace the whole document (parse/open path).
    fn adopt(&mut self, doc: ForgeScene, path: Option<String>) {
        // ids are positional: FIRST_ID + index
        let name_to_id: std::collections::HashMap<String, u64> = doc
            .entities
            .iter()
            .enumerate()
            .map(|(i, e)| (e.name.clone(), FIRST_ID + i as u64))
            .collect();
        self.doc = doc;
        self.ids = (FIRST_ID..FIRST_ID + self.doc.entities.len() as u64).collect();
        self.next_id = FIRST_ID + self.doc.entities.len() as u64;
        self.names = self.doc.entities.iter().map(|e| e.name.clone()).collect();
        self.parents = self
            .doc
            .entities
            .iter()
            .map(|e| e.parent.as_ref().and_then(|n| name_to_id.get(n).copied()))
            .collect();
        self.scene_path = path;
        self.unsynced = false;
        self.mirror_dirty = true;
    }

    /// Main Camera + Directional Light, matching the runtime's default rig.
    fn push_default_rig(&mut self) {
        self.doc = ForgeScene::default();
        self.ids.clear();
        self.parents.clear();
        self.names.clear();
        let cam = SceneEntity {
            name: "Main Camera".into(),
            parent: None,
            kind: SceneEntityKind::Camera,
            transform: ([-4.0, 3.0, 6.0], [-27.0, -33.0, 0.0], [1.0, 1.0, 1.0]),
            visible: true,
            locked: false,
            material: None,
            camera: Some(SceneCamera { fov_deg: 60.0 }),
            light: None,
            scripts: Vec::new(),
        };
        let sun = SceneEntity {
            name: "Directional Light".into(),
            parent: None,
            kind: SceneEntityKind::DirectionalLight,
            transform: ([0.0, 12.0, 0.0], [-90.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
            visible: true,
            locked: false,
            material: None,
            camera: None,
            light: Some(SceneLight::Directional {
                color: [1.0, 1.0, 1.0, 1.0],
                illuminance: 12_000.0,
                shadows: true,
            }),
            scripts: Vec::new(),
        };
        self.doc.entities.push(cam);
        self.doc.entities.push(sun);
        self.reindex();
        self.scene_path = None;
        self.mirror_dirty = true;
    }

    fn reindex(&mut self) {
        self.names = self.doc.entities.iter().map(|e| e.name.clone()).collect();
        self.ids = (FIRST_ID..FIRST_ID + self.doc.entities.len() as u64).collect();
        self.next_id = FIRST_ID + self.doc.entities.len() as u64;
        // parents follow the entity vec for freshly built docs
        if self.parents.len() != self.doc.entities.len() {
            self.parents = vec![None; self.doc.entities.len()];
        }
    }

    // -- lookups ---------------------------------------------------------

    fn index_of(&self, id: u64) -> Option<usize> {
        self.ids.iter().position(|&i| i == id)
    }

    fn unique_name(&self, base: &str) -> String {
        if !self.names.contains(base) {
            return base.to_string();
        }
        for i in 1..10_000u32 {
            let candidate = format!("{base}.{i:03}");
            if !self.names.contains(&candidate) {
                return candidate;
            }
        }
        format!("{base}.dup")
    }

    /// Descendants of `id` (indices), depth-first.
    fn descendants(&self, id: u64) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack: Vec<usize> = self
            .parents
            .iter()
            .enumerate()
            .filter(|(_, p)| **p == Some(id))
            .map(|(i, _)| i)
            .collect();
        while let Some(i) = stack.pop() {
            let cid = self.ids[i];
            out.push(i);
            let kids: Vec<usize> = self
                .parents
                .iter()
                .enumerate()
                .filter(|(_, p)| **p == Some(cid))
                .map(|(k, _)| k)
                .collect();
            stack.extend(kids);
        }
        out
    }

    fn subtree_indices(&self, id: u64) -> Vec<usize> {
        let mut out = self.descendants(id);
        if let Some(i) = self.index_of(id) {
            out.insert(0, i);
        }
        out
    }

    fn default_material() -> SceneMaterial {
        SceneMaterial {
            base_color: [0.65, 0.68, 0.72, 1.0],
            metallic: 0.05,
            roughness: 0.65,
            emissive: [0.0, 0.0, 0.0, 0.0],
        }
    }

    fn rest_height(prim: MeshPrimitive) -> f32 {
        match prim {
            MeshPrimitive::Cube | MeshPrimitive::Sphere | MeshPrimitive::Icosphere => 0.5,
            MeshPrimitive::Capsule | MeshPrimitive::Cylinder | MeshPrimitive::Cone => 1.0,
            MeshPrimitive::Plane => 0.0,
            MeshPrimitive::Torus => 0.25,
        }
    }

    /// Offline equivalent of the runtime's `factory::spawn_entity` defaults.
    fn entity_from_kind(name: &str, kind: &EntityKind) -> SceneEntity {
        match kind {
            EntityKind::Empty => SceneEntity {
                name: name.into(),
                kind: SceneEntityKind::Empty,
                transform: SceneEntity::identity_transform(),
                ..empty_extras()
            },
            EntityKind::Mesh(prim) => SceneEntity {
                name: name.into(),
                kind: SceneEntityKind::Mesh(*prim),
                transform: ([0.0, Self::rest_height(*prim), 0.0], [0.0; 3], [1.0; 3]),
                material: Some(Self::default_material()),
                ..empty_extras()
            },
            EntityKind::Camera => SceneEntity {
                name: name.into(),
                kind: SceneEntityKind::Camera,
                transform: ([0.0, 2.0, 6.0], [0.0; 3], [1.0; 3]),
                camera: Some(SceneCamera { fov_deg: 45.0 }),
                ..empty_extras()
            },
            EntityKind::DirectionalLight => SceneEntity {
                name: name.into(),
                kind: SceneEntityKind::DirectionalLight,
                transform: ([0.0, 12.0, 0.0], [-90.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
                light: Some(SceneLight::Directional {
                    color: [1.0, 1.0, 1.0, 1.0],
                    illuminance: 10_000.0,
                    shadows: true,
                }),
                ..empty_extras()
            },
            EntityKind::PointLight => SceneEntity {
                name: name.into(),
                kind: SceneEntityKind::PointLight,
                transform: ([0.0, 2.5, 0.0], [0.0; 3], [1.0; 3]),
                light: Some(SceneLight::Point {
                    color: [1.0, 1.0, 1.0, 1.0],
                    intensity: 200_000.0,
                    radius: 0.5,
                    shadows: true,
                }),
                ..empty_extras()
            },
            EntityKind::SpotLight => SceneEntity {
                name: name.into(),
                kind: SceneEntityKind::SpotLight,
                transform: ([0.0, 4.0, 0.0], [-90.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
                light: Some(SceneLight::Spot {
                    color: [1.0, 1.0, 1.0, 1.0],
                    intensity: 400_000.0,
                    range: 20.0,
                    outer_angle_deg: 45.0,
                    shadows: true,
                }),
                ..empty_extras()
            },
            EntityKind::PlayerPrefab => SceneEntity {
                name: name.into(),
                kind: SceneEntityKind::Mesh(MeshPrimitive::Capsule),
                transform: ([0.0, 1.0, 0.0], [0.0; 3], [1.0; 3]),
                material: Some(SceneMaterial {
                    base_color: [0.95, 0.62, 0.28, 1.0],
                    metallic: 0.1,
                    roughness: 0.45,
                    emissive: [0.08, 0.03, 0.0, 0.0],
                }),
                scripts: vec![
                    SceneScript::Player { speed: 12.0, jump_force: 25.0, sprint_multiplier: 1.5 },
                    SceneScript::CharacterController {
                        height: 2.0,
                        radius: 0.35,
                        step_offset: 0.5,
                        slope_limit: 45.0,
                    },
                    SceneScript::Health { current: 100.0, max: 100.0 },
                    SceneScript::Inventory { slots: 32 },
                ],
                ..empty_extras()
            },
        }
    }

    // -- command application ----------------------------------------------

    /// Apply one editor command locally. Mirrors the runtime's handlers in
    /// `forge_runtime::commands` / `factory`.
    pub fn apply(&mut self, cmd: &EditorToRuntime) -> Feedback {
        self.mirror_dirty = true;
        match cmd {
            EditorToRuntime::SpawnEntity { name, parent, kind } => {
                let name = self.unique_name(name);
                let entity = Self::entity_from_kind(&name, kind);
                // dangling parents resolve to roots instead of crashing
                let parent = parent.filter(|p| self.index_of(*p).is_some());
                self.doc.entities.push(entity);
                self.ids.push(self.next_id);
                self.parents.push(parent);
                self.names.insert(name);
                let new_id = self.next_id;
                self.next_id += 1;
                self.unsynced = true;
                Feedback { spawned: Some(new_id), ..Feedback::default() }
            }
            EditorToRuntime::DeleteEntity { entity } => {
                let mut doomed = self.subtree_indices(*entity);
                doomed.sort_unstable_by(|a: &usize, b: &usize| b.cmp(a)); // children first
                for i in doomed {
                    if i < self.doc.entities.len() {
                        self.names.remove(&self.doc.entities[i].name);
                        self.doc.entities.remove(i);
                        self.ids.remove(i);
                        self.parents.remove(i);
                    }
                }
                // Reparent orphans of removed interior nodes to roots.
                for p in self.parents.iter_mut() {
                    let orphan = p.map(|pid| pid < self.next_id).unwrap_or(false)
                        && p.map(|pid| !self.ids.contains(&pid)).unwrap_or(false);
                    if orphan {
                        *p = None;
                    }
                }
                self.unsynced = true;
                Feedback::default()
            }
            EditorToRuntime::DuplicateEntity { entity } => {
                let subtree = self.subtree_indices(*entity);
                let mut id_map: Vec<(u64, u64)> = Vec::new();
                let mut spawned = None;
                for &i in &subtree {
                    let mut clone = self.doc.entities[i].clone();
                    let old_id = self.ids[i];
                    let new_id = self.next_id;
                    self.next_id += 1;
                    if old_id == *entity {
                        clone.name = self.unique_name(&clone.name);
                        spawned = Some(new_id);
                    } else {
                        clone.name = self.unique_name(&clone.name);
                    }
                    self.names.insert(clone.name.clone());
                    self.doc.entities.push(clone);
                    self.ids.push(new_id);
                    let parent = self.parents[i].map(|p| {
                        id_map
                            .iter()
                            .find(|(o, _)| *o == p)
                            .map(|(_, n)| *n)
                            .unwrap_or(p)
                    });
                    self.parents.push(parent);
                    id_map.push((old_id, new_id));
                }
                self.unsynced = true;
                Feedback { spawned, ..Feedback::default() }
            }
            EditorToRuntime::Reparent { entity, new_parent } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                // cycle guard
                let mut cursor = *new_parent;
                let mut ok = true;
                while let Some(c) = cursor {
                    if c == *entity {
                        ok = false;
                        break;
                    }
                    cursor = self.index_of(c).and_then(|i| self.parents[i]);
                }
                if ok {
                    self.parents[i] = *new_parent;
                    self.unsynced = true;
                }
                Feedback::default()
            }
            EditorToRuntime::RenameEntity { entity, name } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                let name = self.unique_name(name);
                self.names.remove(&self.doc.entities[i].name);
                self.doc.entities[i].name = name.clone();
                self.names.insert(name);
                self.unsynced = true;
                Feedback::default()
            }
            EditorToRuntime::SetField { entity, component, field, value } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                self.set_field(i, *component, *field, value.clone());
                self.unsynced = true;
                Feedback::default()
            }
            EditorToRuntime::AddComponent { entity, component } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                let e = &mut self.doc.entities[i];
                let label = component.label();
                let added = match component {
                    ComponentKind::Material if e.material.is_none() => {
                        e.material = Some(Self::default_material());
                        true
                    }
                    ComponentKind::Rotator if !has_script(&e.scripts, 0) => {
                        e.scripts.push(SceneScript::Rotator { speed: [0.0, 1.0, 0.0] });
                        true
                    }
                    ComponentKind::Orbiter if !has_script(&e.scripts, 1) => {
                        e.scripts.push(SceneScript::Orbiter {
                            center: [0.0, 0.0, 0.0],
                            radius: 3.0,
                            speed: 1.0,
                        });
                        true
                    }
                    ComponentKind::LinearMover if !has_script(&e.scripts, 2) => {
                        e.scripts.push(SceneScript::LinearMover {
                            velocity: [1.0, 0.0, 0.0],
                            ping_pong: false,
                        });
                        true
                    }
                    ComponentKind::PingPongMover if !has_script(&e.scripts, 3) => {
                        e.scripts.push(SceneScript::PingPongMover {
                            offset: [0.0, 2.0, 0.0],
                            period: 2.0,
                        });
                        true
                    }
                    ComponentKind::Player if !has_script(&e.scripts, 4) => {
                        e.scripts.push(SceneScript::Player {
                            speed: 12.0,
                            jump_force: 25.0,
                            sprint_multiplier: 1.5,
                        });
                        true
                    }
                    ComponentKind::CharacterController if !has_script(&e.scripts, 5) => {
                        e.scripts.push(SceneScript::CharacterController {
                            height: 2.0,
                            radius: 0.35,
                            step_offset: 0.5,
                            slope_limit: 45.0,
                        });
                        true
                    }
                    ComponentKind::Health if !has_script(&e.scripts, 6) => {
                        e.scripts.push(SceneScript::Health { current: 100.0, max: 100.0 });
                        true
                    }
                    ComponentKind::Inventory if !has_script(&e.scripts, 7) => {
                        e.scripts.push(SceneScript::Inventory { slots: 32 });
                        true
                    }
                    _ => false,
                };
                self.unsynced = true;
                let notice = if added {
                    Some((LogLevel::Info, format!("Added {label} (offline)")))
                } else {
                    Some((LogLevel::Warn, format!("{label} is intrinsic or already present (offline)")))
                };
                Feedback { notice, ..Feedback::default() }
            }
            EditorToRuntime::RemoveComponent { entity, component } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                let e = &mut self.doc.entities[i];
                let label = component.label();
                let removed = match component {
                    ComponentKind::Material => e.material.take().is_some(),
                    ComponentKind::Rotator => remove_script(&mut e.scripts, 0),
                    ComponentKind::Orbiter => remove_script(&mut e.scripts, 1),
                    ComponentKind::LinearMover => remove_script(&mut e.scripts, 2),
                    ComponentKind::PingPongMover => remove_script(&mut e.scripts, 3),
                    ComponentKind::Player => remove_script(&mut e.scripts, 4),
                    ComponentKind::CharacterController => remove_script(&mut e.scripts, 5),
                    ComponentKind::Health => remove_script(&mut e.scripts, 6),
                    ComponentKind::Inventory => remove_script(&mut e.scripts, 7),
                    _ => false,
                };
                self.unsynced = true;
                let notice = if removed {
                    Some((LogLevel::Info, format!("Removed {label} (offline)")))
                } else {
                    Some((LogLevel::Warn, format!("{label} is intrinsic or missing (offline)")))
                };
                Feedback { notice, ..Feedback::default() }
            }
            EditorToRuntime::NewScene => {
                self.push_default_rig();
                self.unsynced = true;
                Feedback {
                    notice: Some((LogLevel::Info, "New scene created (offline)".into())),
                    ..Feedback::default()
                }
            }
            EditorToRuntime::OpenScene { path } => match std::fs::read_to_string(path) {
                Ok(text) => match ron::from_str::<ForgeScene>(&text) {
                    Ok(doc) => {
                        let count = doc.entities.len();
                        self.adopt(doc, Some(path.clone()));
                        self.unsynced = true;
                        Feedback {
                            notice: Some((LogLevel::Info, format!("Opened {path} ({count} entities), offline"))),
                            ..Feedback::default()
                        }
                    }
                    Err(e) => Feedback {
                        notice: Some((LogLevel::Error, format!("Open failed: {e:#}"))),
                        ..Feedback::default()
                    },
                },
                Err(e) => Feedback {
                    notice: Some((LogLevel::Error, format!("Open failed: {e:#}"))),
                    ..Feedback::default()
                },
            },
            EditorToRuntime::SaveScene { path } => match self.save_to(path) {
                Ok(count) => {
                    self.scene_path = Some(path.clone());
                    self.unsynced = false; // the file now matches the document
                    Feedback {
                        notice: Some((LogLevel::Info, format!("Saved {path} ({count} entities), offline"))),
                        ..Feedback::default()
                    }
                }
                Err(e) => Feedback {
                    notice: Some((LogLevel::Error, format!("Save failed: {e:#}"))),
                    ..Feedback::default()
                },
            },
            EditorToRuntime::MoveEntity { entity, delta } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                let t = &mut self.doc.entities[i].transform;
                for k in 0..3 {
                    t.0[k] += delta[k];
                }
                self.unsynced = true;
                Feedback::default()
            }
            EditorToRuntime::RotateEntityWorld { entity, axis, angle_deg } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                let rad = angle_deg.to_radians();
                let t = &mut self.doc.entities[i].transform;
                let [ex, ey, ez] = t.1;
                let q = Quat::from_axis_angle(*axis, rad)
                    .mul(Quat::from_euler_xyz(ex.to_radians(), ey.to_radians(), ez.to_radians()));
                let [nx, ny, nz] = q.to_euler_xyz();
                t.1 = [nx.to_degrees(), ny.to_degrees(), nz.to_degrees()];
                self.unsynced = true;
                Feedback::default()
            }
            EditorToRuntime::ScaleEntityBy { entity, factor } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                let t = &mut self.doc.entities[i].transform;
                for k in 0..3 {
                    t.2[k] = (t.2[k] * factor[k]).max(0.001);
                }
                self.unsynced = true;
                Feedback::default()
            }
            EditorToRuntime::BeginGizmoGesture { entity } => {
                if let Some(i) = self.index_of(*entity) {
                    self.gesture = Some(Gesture { id: *entity, pre: self.doc.entities[i].transform });
                }
                Feedback::default()
            }
            EditorToRuntime::EndGizmoGesture { entity, label } => {
                if let Some(g) = self.gesture.take() {
                    if g.id == *entity {
                        if let Some(i) = self.index_of(*entity) {
                            let post = self.doc.entities[i].transform;
                            if g.pre != post {
                                let fields = |t: ([f32; 3], [f32; 3], [f32; 3])| {
                                    transform_cmds(*entity, t)
                                };
                                return Feedback {
                                    gesture_done: Some((label.clone(), fields(g.pre), fields(post))),
                                    ..Feedback::default()
                                };
                            }
                        }
                    }
                }
                Feedback::default()
            }
            EditorToRuntime::SetLocked { entity, locked } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                self.doc.entities[i].locked = *locked;
                self.unsynced = true;
                Feedback::default()
            }
            EditorToRuntime::SetEnvironment(settings) => {
                self.doc.environment = settings.clone();
                self.unsynced = true;
                Feedback::default()
            }
            EditorToRuntime::SetAnimTime(t) => {
                self.anim.time = *t;
                Feedback::default()
            }
            EditorToRuntime::SetAnimPlaying(p) => {
                self.anim.playing = *p;
                Feedback::default()
            }
            EditorToRuntime::SetAnimDuration(d) => {
                self.anim.duration = *d;
                self.doc.animation.duration = *d;
                self.unsynced = true;
                Feedback::default()
            }
            EditorToRuntime::SetAnimLooped(l) => {
                self.anim.looped = *l;
                Feedback::default()
            }
            EditorToRuntime::AddKeyframe { entity, track, time, value } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                let name = self.doc.entities[i].name.clone();
                let entries = &mut self.doc.animation.entries;
                let entry = match entries.iter_mut().find(|e| e.name == name) {
                    Some(e) => e,
                    None => {
                        entries.push(SceneAnimEntry { name: name.clone(), tracks: Vec::new() });
                        entries.last_mut().expect("just pushed")
                    }
                };
                let slot = entry.tracks.iter_mut().find(|(t, _)| *t == *track);
                match slot {
                    Some((_, keys)) => {
                        keys.push((*time, *value));
                        keys.sort_by(|a, b| a.0.total_cmp(&b.0));
                    }
                    None => entry.tracks.push((*track, Vec::new())),
                }
                self.unsynced = true;
                Feedback::default()
            }
            EditorToRuntime::RemoveKeyframe { entity, track, index } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                let name = self.doc.entities[i].name.clone();
                if let Some(entry) = self.doc.animation.entries.iter_mut().find(|e| e.name == name) {
                    if let Some((_, keys)) = entry.tracks.iter_mut().find(|(t, _)| t == track) {
                        if *index < keys.len() {
                            keys.remove(*index);
                            self.unsynced = true;
                        }
                    }
                }
                Feedback::default()
            }
            EditorToRuntime::MoveKeyframe { entity, track, index, new_time } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                let name = self.doc.entities[i].name.clone();
                if let Some(entry) = self.doc.animation.entries.iter_mut().find(|e| e.name == name) {
                    if let Some((_, keys)) = entry.tracks.iter_mut().find(|(t, _)| t == track) {
                        if *index < keys.len() {
                            let v = keys.remove(*index);
                            keys.push((*new_time, v.1));
                            keys.sort_by(|a, b| a.0.total_cmp(&b.0));
                            self.unsynced = true;
                        }
                    }
                }
                Feedback::default()
            }
            EditorToRuntime::ClearTracks { entity } => {
                let Some(i) = self.index_of(*entity) else { return Feedback::default() };
                let name = self.doc.entities[i].name.clone();
                self.doc.animation.entries.retain(|e| e.name != name);
                self.unsynced = true;
                Feedback::default()
            }
            // --- engine-only commands (honest no-ops offline) -------------
            EditorToRuntime::Hello
            | EditorToRuntime::Ping(_)
            | EditorToRuntime::SetViewportSize { .. }
            | EditorToRuntime::Pick { .. }
            | EditorToRuntime::RequestScreenshot { .. }
            | EditorToRuntime::RequestFullState
            | EditorToRuntime::Shutdown
            | EditorToRuntime::LoadSceneDoc { .. }
            | EditorToRuntime::SetEditorCamera { .. }
            | EditorToRuntime::SetViewportCamera { .. } => Feedback {
                needs_engine: true,
                ..Feedback::default()
            },
            EditorToRuntime::SetPlayMode { .. } => Feedback {
                needs_engine: true,
                notice: Some((
                    LogLevel::Warn,
                    "Play mode needs the render engine — scene edits below still work".into(),
                )),
                ..Feedback::default()
            },
            // Select is tracked by the editor state itself.
            EditorToRuntime::Select { .. } => Feedback::default(),
        }
    }

    fn set_field(&mut self, i: usize, component: ComponentKind, field: ComponentField, value: FieldValue) {
        let f32_of = |v: &FieldValue| match v {
            FieldValue::F32(f) => Some(*f),
            _ => None,
        };
        let e = &mut self.doc.entities[i];
        match (component, field) {
            (ComponentKind::Transform, ComponentField::Translation) => {
                if let FieldValue::Vec3(v) = value {
                    e.transform.0 = v;
                }
            }
            (ComponentKind::Transform, ComponentField::RotationEulerDeg) => {
                if let FieldValue::Vec3(v) = value {
                    e.transform.1 = v;
                }
            }
            (ComponentKind::Transform, ComponentField::Scale) => {
                if let FieldValue::Vec3(v) = value {
                    e.transform.2 = v;
                }
            }
            (ComponentKind::Visibility, ComponentField::EntityVisible) => {
                e.visible = matches!(value, FieldValue::Bool(true));
            }
            (ComponentKind::Mesh, ComponentField::MeshPrimitiveKind) => {
                if let FieldValue::Mesh(prim) = value {
                    e.kind = SceneEntityKind::Mesh(prim);
                }
            }
            (ComponentKind::Material, _) => {
                let Some(mat) = e.material.as_mut() else { return };
                match field {
                    ComponentField::BaseColor => {
                        if let FieldValue::Rgba(c) = value {
                            mat.base_color = c;
                        }
                    }
                    ComponentField::Metallic => {
                        if let Some(f) = f32_of(&value) {
                            mat.metallic = f.clamp(0.0, 1.0);
                        }
                    }
                    ComponentField::Roughness => {
                        if let Some(f) = f32_of(&value) {
                            mat.roughness = f.clamp(0.0, 1.0);
                        }
                    }
                    ComponentField::Emissive => {
                        if let FieldValue::Rgba(c) = value {
                            mat.emissive = c;
                        }
                    }
                    _ => {}
                }
            }
            (ComponentKind::Camera, ComponentField::FovDeg) => {
                if let (Some(cam), Some(f)) = (e.camera.as_mut(), f32_of(&value)) {
                    cam.fov_deg = f.clamp(1.0, 150.0);
                }
            }
            (ComponentKind::DirectionalLight, _) => {
                let Some(SceneLight::Directional { color, illuminance, shadows }) = e.light.as_mut() else {
                    return;
                };
                match field {
                    ComponentField::SunColor => {
                        if let FieldValue::Rgba(c) = value {
                            *color = c;
                        }
                    }
                    ComponentField::SunIlluminance => {
                        if let Some(f) = f32_of(&value) {
                            *illuminance = f.max(0.0);
                        }
                    }
                    ComponentField::SunShadows => {
                        if let FieldValue::Bool(b) = value {
                            *shadows = b;
                        }
                    }
                    _ => {}
                }
            }
            (ComponentKind::PointLight, _) => {
                let Some(SceneLight::Point { color, intensity, radius, shadows }) = e.light.as_mut() else {
                    return;
                };
                match field {
                    ComponentField::LightColor => {
                        if let FieldValue::Rgba(c) = value {
                            *color = c;
                        }
                    }
                    ComponentField::LightIntensity => {
                        if let Some(f) = f32_of(&value) {
                            *intensity = f.max(0.0);
                        }
                    }
                    ComponentField::LightRadius => {
                        if let Some(f) = f32_of(&value) {
                            *radius = f.max(0.0);
                        }
                    }
                    ComponentField::LightShadows => {
                        if let FieldValue::Bool(b) = value {
                            *shadows = b;
                        }
                    }
                    _ => {}
                }
            }
            (ComponentKind::SpotLight, _) => {
                let Some(SceneLight::Spot { color, intensity, range, outer_angle_deg, shadows }) =
                    e.light.as_mut()
                else {
                    return;
                };
                match field {
                    ComponentField::LightColor => {
                        if let FieldValue::Rgba(c) = value {
                            *color = c;
                        }
                    }
                    ComponentField::LightIntensity => {
                        if let Some(f) = f32_of(&value) {
                            *intensity = f.max(0.0);
                        }
                    }
                    ComponentField::LightRange => {
                        if let Some(f) = f32_of(&value) {
                            *range = f.max(0.0);
                        }
                    }
                    ComponentField::LightOuterAngleDeg => {
                        if let Some(f) = f32_of(&value) {
                            *outer_angle_deg = f.clamp(1.0, 89.0);
                        }
                    }
                    ComponentField::LightShadows => {
                        if let FieldValue::Bool(b) = value {
                            *shadows = b;
                        }
                    }
                    _ => {}
                }
            }
            (_, _) => apply_script_field(e, component, field, value),
        }
    }

    // -- persistence -----------------------------------------------------

    pub fn save_to(&self, path: &str) -> anyhow::Result<usize> {
        let scene = self.to_scene();
        let count = scene.entities.len();
        let text = ron::ser::to_string_pretty(&scene, ron::ser::PrettyConfig::default())?;
        std::fs::write(path, text)?;
        Ok(count)
    }

    /// The serialisable document (parents back to names, ids dropped).
    pub fn to_scene(&self) -> ForgeScene {
        let mut scene = self.doc.clone();
        for (i, entity) in scene.entities.iter_mut().enumerate() {
            entity.parent = self.parents[i].and_then(|p| {
                self.index_of(p).map(|pi| self.doc.entities[pi].name.clone())
            });
        }
        scene.animation.duration = if self.anim.duration > 0.0 { self.anim.duration } else { scene.animation.duration };
        scene
    }

    // -- mirrors for the UI ----------------------------------------------

    pub fn scene_path(&self) -> Option<String> {
        self.scene_path.clone()
    }

    pub fn has_unsynced_edits(&self) -> bool {
        self.unsynced
    }

    pub fn take_mirror_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.mirror_dirty, false)
    }

    /// Flattened hierarchy, depth-first, mirroring the runtime's snapshot.
    pub fn hierarchy(&self, selected: Option<u64>) -> Vec<HierNode> {
        let children_of = |id: u64| -> Vec<usize> {
            (0..self.doc.entities.len())
                .filter(|&i| self.parents.get(i).copied().flatten() == Some(id))
                .collect()
        };
        let mut out = Vec::new();
        let mut stack: Vec<(usize, u32)> = (0..self.doc.entities.len())
            .filter(|&i| self.parents.get(i).copied().flatten().is_none())
            .rev()
            .map(|i| (i, 0))
            .collect();
        let mut visited: HashSet<usize> = HashSet::new();
        while let Some((i, depth)) = stack.pop() {
            if depth > 64 || !visited.insert(i) {
                continue;
            }
            let e = &self.doc.entities[i];
            let id = self.ids[i];
            out.push(HierNode {
                id,
                name: e.name.clone(),
                icon: icon_of(e),
                visible: e.visible,
                locked: e.locked,
                has_children: !children_of(id).is_empty(),
                depth,
                selected: selected == Some(id),
            });
            let kids: Vec<(usize, u32)> = children_of(id).into_iter().map(|k| (k, depth + 1)).collect();
            // reverse so pop order = document order
            stack.extend(kids.into_iter().rev());
        }
        out
    }

    /// Inspector payload for one entity — row labels/units identical to the
    /// runtime's `factory::extract_components`.
    pub fn components_for(&self, id: u64) -> Option<(String, Vec<ComponentData>)> {
        let i = self.index_of(id)?;
        let e = &self.doc.entities[i];
        let mut components: Vec<ComponentData> = Vec::new();

        components.push(ComponentData {
            kind: ComponentKind::Transform,
            rows: vec![
                (
                    ComponentField::Translation,
                    FieldRow { label: "Translation".into(), value: FieldValue::Vec3(e.transform.0), unit: None },
                ),
                (
                    ComponentField::RotationEulerDeg,
                    FieldRow {
                        label: "Rotation".into(),
                        value: FieldValue::Vec3(e.transform.1),
                        unit: Some("deg".into()),
                    },
                ),
                (
                    ComponentField::Scale,
                    FieldRow { label: "Scale".into(), value: FieldValue::Vec3(e.transform.2), unit: None },
                ),
            ],
        });

        components.push(ComponentData {
            kind: ComponentKind::Visibility,
            rows: vec![(
                ComponentField::EntityVisible,
                FieldRow { label: "Visible".into(), value: FieldValue::Bool(e.visible), unit: None },
            )],
        });

        if let SceneEntityKind::Mesh(prim) = &e.kind {
            components.push(ComponentData {
                kind: ComponentKind::Mesh,
                rows: vec![(
                    ComponentField::MeshPrimitiveKind,
                    FieldRow { label: "Primitive".into(), value: FieldValue::Mesh(*prim), unit: None },
                )],
            });
        }

        if let Some(m) = &e.material {
            components.push(ComponentData {
                kind: ComponentKind::Material,
                rows: vec![
                    (
                        ComponentField::BaseColor,
                        FieldRow { label: "Base Color".into(), value: FieldValue::Rgba(m.base_color), unit: None },
                    ),
                    (
                        ComponentField::Metallic,
                        FieldRow { label: "Metallic".into(), value: FieldValue::F32(m.metallic), unit: None },
                    ),
                    (
                        ComponentField::Roughness,
                        FieldRow { label: "Roughness".into(), value: FieldValue::F32(m.roughness), unit: None },
                    ),
                    (
                        ComponentField::Emissive,
                        FieldRow { label: "Emissive".into(), value: FieldValue::Rgba(m.emissive), unit: None },
                    ),
                ],
            });
        }

        if let Some(cam) = &e.camera {
            components.push(ComponentData {
                kind: ComponentKind::Camera,
                rows: vec![
                    (
                        ComponentField::FovDeg,
                        FieldRow {
                            label: "FOV".into(),
                            value: FieldValue::F32(cam.fov_deg),
                            unit: Some("deg".into()),
                        },
                    ),
                    (
                        ComponentField::CameraHdr,
                        FieldRow { label: "HDR".into(), value: FieldValue::Bool(false), unit: None },
                    ),
                ],
            });
        }

        match &e.light {
            Some(SceneLight::Directional { color, illuminance, shadows }) => {
                components.push(ComponentData {
                    kind: ComponentKind::DirectionalLight,
                    rows: vec![
                        (
                            ComponentField::SunColor,
                            FieldRow { label: "Color".into(), value: FieldValue::Rgba(*color), unit: None },
                        ),
                        (
                            ComponentField::SunIlluminance,
                            FieldRow {
                                label: "Illuminance".into(),
                                value: FieldValue::F32(*illuminance),
                                unit: Some("lux".into()),
                            },
                        ),
                        (
                            ComponentField::SunShadows,
                            FieldRow { label: "Shadow Maps".into(), value: FieldValue::Bool(*shadows), unit: None },
                        ),
                    ],
                });
            }
            Some(SceneLight::Point { color, intensity, radius, shadows }) => {
                components.push(ComponentData {
                    kind: ComponentKind::PointLight,
                    rows: vec![
                        (
                            ComponentField::LightColor,
                            FieldRow { label: "Color".into(), value: FieldValue::Rgba(*color), unit: None },
                        ),
                        (
                            ComponentField::LightIntensity,
                            FieldRow {
                                label: "Intensity".into(),
                                value: FieldValue::F32(*intensity),
                                unit: Some("lm".into()),
                            },
                        ),
                        (
                            ComponentField::LightRadius,
                            FieldRow { label: "Radius".into(), value: FieldValue::F32(*radius), unit: None },
                        ),
                        (
                            ComponentField::LightShadows,
                            FieldRow { label: "Shadow Maps".into(), value: FieldValue::Bool(*shadows), unit: None },
                        ),
                    ],
                });
            }
            Some(SceneLight::Spot { color, intensity, range, outer_angle_deg, shadows }) => {
                components.push(ComponentData {
                    kind: ComponentKind::SpotLight,
                    rows: vec![
                        (
                            ComponentField::LightColor,
                            FieldRow { label: "Color".into(), value: FieldValue::Rgba(*color), unit: None },
                        ),
                        (
                            ComponentField::LightIntensity,
                            FieldRow {
                                label: "Intensity".into(),
                                value: FieldValue::F32(*intensity),
                                unit: Some("lm".into()),
                            },
                        ),
                        (
                            ComponentField::LightRange,
                            FieldRow { label: "Range".into(), value: FieldValue::F32(*range), unit: None },
                        ),
                        (
                            ComponentField::LightOuterAngleDeg,
                            FieldRow {
                                label: "Outer Angle".into(),
                                value: FieldValue::F32(*outer_angle_deg),
                                unit: Some("deg".into()),
                            },
                        ),
                        (
                            ComponentField::LightShadows,
                            FieldRow { label: "Shadow Maps".into(), value: FieldValue::Bool(*shadows), unit: None },
                        ),
                    ],
                });
            }
            None => {}
        }

        for script in &e.scripts {
            push_script_rows(script, &mut components);
        }

        Some((e.name.clone(), components))
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn empty_extras() -> SceneEntity {
    SceneEntity {
        name: String::new(),
        parent: None,
        kind: SceneEntityKind::Empty,
        transform: ([0.0; 3], [0.0; 3], [1.0; 3]),
        visible: true,
        locked: false,
        material: None,
        camera: None,
        light: None,
        scripts: Vec::new(),
    }
}

fn icon_of(e: &SceneEntity) -> NodeIcon {
    match &e.kind {
        SceneEntityKind::Camera => NodeIcon::Camera,
        SceneEntityKind::DirectionalLight | SceneEntityKind::PointLight | SceneEntityKind::SpotLight => {
            NodeIcon::Light
        }
        SceneEntityKind::Mesh(_) => {
            if e.scripts.iter().any(is_player) {
                NodeIcon::Player
            } else {
                NodeIcon::Mesh
            }
        }
        SceneEntityKind::Empty => {
            if !e.scripts.is_empty() {
                NodeIcon::Script
            } else {
                NodeIcon::Group
            }
        }
    }
}

fn is_player(s: &SceneScript) -> bool {
    matches!(s, SceneScript::Player { .. })
}

/// Script discrimination by kind index (the doc enums carry data, not tags we
/// can compare directly).
fn script_index(s: &SceneScript) -> u8 {
    match s {
        SceneScript::Rotator { .. } => 0,
        SceneScript::Orbiter { .. } => 1,
        SceneScript::LinearMover { .. } => 2,
        SceneScript::PingPongMover { .. } => 3,
        SceneScript::Player { .. } => 4,
        SceneScript::CharacterController { .. } => 5,
        SceneScript::Health { .. } => 6,
        SceneScript::Inventory { .. } => 7,
    }
}

fn has_script(scripts: &[SceneScript], kind: u8) -> bool {
    scripts.iter().any(|s| script_index(s) == kind)
}

fn remove_script(scripts: &mut Vec<SceneScript>, kind: u8) -> bool {
    let before = scripts.len();
    scripts.retain(|s| script_index(s) != kind);
    scripts.len() != before
}

fn transform_cmds(entity: u64, t: ([f32; 3], [f32; 3], [f32; 3])) -> Vec<EditorToRuntime> {
    use forge_ipc::{ComponentField as F, ComponentKind as K, FieldValue as V};
    vec![
        EditorToRuntime::SetField {
            entity,
            component: K::Transform,
            field: F::Translation,
            value: V::Vec3(t.0),
        },
        EditorToRuntime::SetField {
            entity,
            component: K::Transform,
            field: F::RotationEulerDeg,
            value: V::Vec3(t.1),
        },
        EditorToRuntime::SetField {
            entity,
            component: K::Transform,
            field: F::Scale,
            value: V::Vec3(t.2),
        },
    ]
}

fn apply_script_field(
    e: &mut SceneEntity,
    component: ComponentKind,
    field: ComponentField,
    value: FieldValue,
) {
    let f32_of = |v: &FieldValue| match v {
        FieldValue::F32(f) => Some(*f),
        _ => None,
    };
    let v3 = |v: &FieldValue| match v {
        FieldValue::Vec3(a) => Some(*a),
        _ => None,
    };
    for script in e.scripts.iter_mut() {
        match (script_index(script), &component, &field) {
            (0, ComponentKind::Rotator, ComponentField::RotatorSpeed) => {
                if let Some(a) = v3(&value) {
                    if let SceneScript::Rotator { speed } = script {
                        *speed = a;
                    }
                }
            }
            (1, ComponentKind::Orbiter, ComponentField::OrbiterCenter) => {
                if let Some(a) = v3(&value) {
                    if let SceneScript::Orbiter { center, .. } = script {
                        *center = a;
                    }
                }
            }
            (1, ComponentKind::Orbiter, ComponentField::OrbiterRadius) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::Orbiter { radius, .. } = script {
                        *radius = f;
                    }
                }
            }
            (1, ComponentKind::Orbiter, ComponentField::OrbiterSpeed) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::Orbiter { speed, .. } = script {
                        *speed = f;
                    }
                }
            }
            (2, ComponentKind::LinearMover, ComponentField::MoverVelocity) => {
                if let Some(a) = v3(&value) {
                    if let SceneScript::LinearMover { velocity, .. } = script {
                        *velocity = a;
                    }
                }
            }
            (2, ComponentKind::LinearMover, ComponentField::MoverPingPong) => {
                if let FieldValue::Bool(b) = value {
                    if let SceneScript::LinearMover { ping_pong, .. } = script {
                        *ping_pong = b;
                    }
                }
            }
            (3, ComponentKind::PingPongMover, ComponentField::PingPongOffset) => {
                if let Some(a) = v3(&value) {
                    if let SceneScript::PingPongMover { offset, .. } = script {
                        *offset = a;
                    }
                }
            }
            (3, ComponentKind::PingPongMover, ComponentField::PingPongPeriod) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::PingPongMover { period, .. } = script {
                        *period = f;
                    }
                }
            }
            (4, ComponentKind::Player, ComponentField::PlayerSpeed) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::Player { speed, .. } = script {
                        *speed = f;
                    }
                }
            }
            (4, ComponentKind::Player, ComponentField::PlayerJumpForce) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::Player { jump_force, .. } = script {
                        *jump_force = f;
                    }
                }
            }
            (4, ComponentKind::Player, ComponentField::PlayerSprintMultiplier) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::Player { sprint_multiplier, .. } = script {
                        *sprint_multiplier = f;
                    }
                }
            }
            (5, ComponentKind::CharacterController, ComponentField::CcHeight) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::CharacterController { height, .. } = script {
                        *height = f;
                    }
                }
            }
            (5, ComponentKind::CharacterController, ComponentField::CcRadius) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::CharacterController { radius, .. } = script {
                        *radius = f;
                    }
                }
            }
            (5, ComponentKind::CharacterController, ComponentField::CcStepOffset) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::CharacterController { step_offset, .. } = script {
                        *step_offset = f;
                    }
                }
            }
            (5, ComponentKind::CharacterController, ComponentField::CcSlopeLimit) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::CharacterController { slope_limit, .. } = script {
                        *slope_limit = f;
                    }
                }
            }
            (6, ComponentKind::Health, ComponentField::HealthCurrent) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::Health { current, .. } = script {
                        *current = f;
                    }
                }
            }
            (6, ComponentKind::Health, ComponentField::HealthMax) => {
                if let Some(f) = f32_of(&value) {
                    if let SceneScript::Health { max, .. } = script {
                        *max = f;
                    }
                }
            }
            (7, ComponentKind::Inventory, ComponentField::InventorySlots) => {
                if let FieldValue::U32(u) = value {
                    if let SceneScript::Inventory { slots } = script {
                        *slots = u;
                    }
                }
            }
            _ => {}
        }
    }
}

fn push_script_rows(script: &SceneScript, components: &mut Vec<ComponentData>) {
    let (kind, rows): (ComponentKind, Vec<(ComponentField, FieldRow)>) = match script {
        SceneScript::Rotator { speed } => (
            ComponentKind::Rotator,
            vec![(
                ComponentField::RotatorSpeed,
                FieldRow { label: "Speed (rad/s)".into(), value: FieldValue::Vec3(*speed), unit: None },
            )],
        ),
        SceneScript::Orbiter { center, radius, speed } => (
            ComponentKind::Orbiter,
            vec![
                (
                    ComponentField::OrbiterCenter,
                    FieldRow { label: "Center".into(), value: FieldValue::Vec3(*center), unit: None },
                ),
                (
                    ComponentField::OrbiterRadius,
                    FieldRow { label: "Radius".into(), value: FieldValue::F32(*radius), unit: None },
                ),
                (
                    ComponentField::OrbiterSpeed,
                    FieldRow {
                        label: "Speed".into(),
                        value: FieldValue::F32(*speed),
                        unit: Some("rad/s".into()),
                    },
                ),
            ],
        ),
        SceneScript::LinearMover { velocity, ping_pong } => (
            ComponentKind::LinearMover,
            vec![
                (
                    ComponentField::MoverVelocity,
                    FieldRow { label: "Velocity".into(), value: FieldValue::Vec3(*velocity), unit: None },
                ),
                (
                    ComponentField::MoverPingPong,
                    FieldRow { label: "Ping Pong".into(), value: FieldValue::Bool(*ping_pong), unit: None },
                ),
            ],
        ),
        SceneScript::PingPongMover { offset, period } => (
            ComponentKind::PingPongMover,
            vec![
                (
                    ComponentField::PingPongOffset,
                    FieldRow { label: "Offset".into(), value: FieldValue::Vec3(*offset), unit: None },
                ),
                (
                    ComponentField::PingPongPeriod,
                    FieldRow {
                        label: "Period".into(),
                        value: FieldValue::F32(*period),
                        unit: Some("s".into()),
                    },
                ),
            ],
        ),
        SceneScript::Player { speed, jump_force, sprint_multiplier } => (
            ComponentKind::Player,
            vec![
                (
                    ComponentField::PlayerSpeed,
                    FieldRow { label: "speed".into(), value: FieldValue::F32(*speed), unit: None },
                ),
                (
                    ComponentField::PlayerJumpForce,
                    FieldRow { label: "jump_force".into(), value: FieldValue::F32(*jump_force), unit: None },
                ),
                (
                    ComponentField::PlayerSprintMultiplier,
                    FieldRow {
                        label: "sprint_multiplier".into(),
                        value: FieldValue::F32(*sprint_multiplier),
                        unit: None,
                    },
                ),
            ],
        ),
        SceneScript::CharacterController { height, radius, step_offset, slope_limit } => (
            ComponentKind::CharacterController,
            vec![
                (
                    ComponentField::CcHeight,
                    FieldRow { label: "height".into(), value: FieldValue::F32(*height), unit: None },
                ),
                (
                    ComponentField::CcRadius,
                    FieldRow { label: "radius".into(), value: FieldValue::F32(*radius), unit: None },
                ),
                (
                    ComponentField::CcStepOffset,
                    FieldRow { label: "step_offset".into(), value: FieldValue::F32(*step_offset), unit: None },
                ),
                (
                    ComponentField::CcSlopeLimit,
                    FieldRow {
                        label: "slope_limit".into(),
                        value: FieldValue::F32(*slope_limit),
                        unit: Some("deg".into()),
                    },
                ),
            ],
        ),
        SceneScript::Health { current, max } => (
            ComponentKind::Health,
            vec![
                (
                    ComponentField::HealthCurrent,
                    FieldRow { label: "current".into(), value: FieldValue::F32(*current), unit: None },
                ),
                (
                    ComponentField::HealthMax,
                    FieldRow { label: "max".into(), value: FieldValue::F32(*max), unit: None },
                ),
            ],
        ),
        SceneScript::Inventory { slots } => (
            ComponentKind::Inventory,
            vec![(
                ComponentField::InventorySlots,
                FieldRow { label: "slots".into(), value: FieldValue::U32(*slots), unit: None },
            )],
        ),
    };
    components.push(ComponentData { kind, rows });
}

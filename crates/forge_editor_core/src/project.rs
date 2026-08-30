//! On-disk BevyForge project model.
//!
//! A project is a directory with this layout:
//!
//! ```text
//! MyProject/
//! ├── BevyForge.toml          # manifest (name, editor version)
//! ├── assets/
//! │   ├── scenes/*.scn.ron    # Bevy scenes (DynamicScene RON)
//! │   ├── scripts/*.rs        # mirrors of forge_scripts sources
//! │   ├── prefabs/
//! │   ├── materials/
//! │   ├── textures/
//! │   └── meshes/
//! └── scripts/                # editable script sources (compiled crate)
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Parsed `BevyForge.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub name: String,
    /// Engine feature level the project was created with.
    pub engine: String,
    #[serde(default)]
    pub main_scene: String,
}

impl Default for ProjectManifest {
    fn default() -> Self {
        Self {
            name: "Untitled Project".into(),
            engine: "bevy 0.19".into(),
            main_scene: "assets/scenes/main.scn.ron".into(),
        }
    }
}

/// Opened project with resolved paths.
#[derive(Debug, Clone)]
pub struct Project {
    pub root: PathBuf,
    pub manifest: ProjectManifest,
}

impl Project {
    /// Open an existing project directory, creating the manifest if missing.
    pub fn open(root: &Path) -> Result<Self> {
        if !root.is_dir() {
            bail!("project path is not a directory: {}", root.display());
        }
        let manifest_path = root.join("BevyForge.toml");
        let manifest = if manifest_path.exists() {
            let raw = fs::read_to_string(&manifest_path)
                .with_context(|| format!("reading {}", manifest_path.display()))?;
            toml::from_str(&raw).context("parsing BevyForge.toml")?
        } else {
            ProjectManifest::default()
        };
        Ok(Self { root: root.to_path_buf(), manifest })
    }

    /// Create a fresh project (directories + manifest + starter scene folder).
    pub fn create(root: &Path, name: &str) -> Result<Self> {
        fs::create_dir_all(root)?;
        for sub in [
            "assets/scenes",
            "assets/scripts",
            "assets/prefabs",
            "assets/materials",
            "assets/textures",
            "assets/meshes",
            "scripts",
        ] {
            fs::create_dir_all(root.join(sub))?;
        }
        let manifest = ProjectManifest {
            name: name.to_string(),
            ..ProjectManifest::default()
        };
        let manifest_path = root.join("BevyForge.toml");
        fs::write(&manifest_path, toml::to_string_pretty(&manifest)?)?;
        Ok(Self { root: root.to_path_buf(), manifest })
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("BevyForge.toml")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn scenes_dir(&self) -> PathBuf {
        self.assets_dir().join("scenes")
    }

    pub fn scripts_crate_dir(&self) -> PathBuf {
        self.root.join("scripts")
    }

    /// Persist the manifest (e.g. after renaming the project).
    pub fn save_manifest(&self) -> Result<()> {
        fs::write(self.manifest_path(), toml::to_string_pretty(&self.manifest)?)?;
        Ok(())
    }

    /// Resolve a scene path against the project; `""` maps to the main scene.
    pub fn resolve_scene(&self, scene: &str) -> PathBuf {
        if scene.is_empty() {
            self.root.join(&self.manifest.main_scene)
        } else if scene.starts_with("assets/") {
            self.root.join(scene)
        } else {
            self.scenes_dir().join(scene)
        }
    }

    /// All `*.scn.ron` files under assets/scenes, sorted, project-relative.
    pub fn list_scenes(&self) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Ok(entries) = fs::read_dir(self.scenes_dir()) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().map(|e| e == "ron").unwrap_or(false) {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_open_roundtrip() {
        let dir = std::env::temp_dir().join(format!("forge_proj_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let proj = Project::create(&dir, "Roundtrip").unwrap();
        assert_eq!(proj.manifest.name, "Roundtrip");
        assert!(dir.join("assets/scenes").is_dir());
        let reopened = Project::open(&dir).unwrap();
        assert_eq!(reopened.manifest.name, "Roundtrip");
        let _ = fs::remove_dir_all(&dir);
    }
}

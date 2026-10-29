use std::path::Path;

use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::draw::{self, Camera};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stroke {
    pub points: Vec<[f32; 2]>,
    pub color: [f32; 4],
    pub width: f32,
}

#[derive(Clone, Debug)]
enum Action {
    Add(Stroke),
    Remove(Vec<(usize, Stroke)>),
    Clear(Vec<Stroke>),
}

#[derive(Clone, Debug)]
pub struct Document {
    pub strokes: Vec<Stroke>,
    undo: Vec<Action>,
    redo: Vec<Action>,
    pub revision: u64,
    pub dirty: bool,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            strokes: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            revision: 0,
            dirty: false,
        }
    }
}

impl Document {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn add_stroke(&mut self, stroke: Stroke) {
        if stroke.points.is_empty() {
            return;
        }
        self.strokes.push(stroke.clone());
        self.push(Action::Add(stroke));
    }

    pub fn extract_hits(&mut self, samples: &[Vec2], radius: f32) -> Vec<(usize, Stroke)> {
        let mut removed = Vec::new();
        for p in samples {
            let mut i = 0;
            while i < self.strokes.len() {
                let stroke = &self.strokes[i];
                if draw::hits_polyline(&stroke.points, stroke.width, *p, radius) {
                    removed.push((i, self.strokes.remove(i)));
                } else {
                    i += 1;
                }
            }
        }
        if !removed.is_empty() {
            self.revision = self.revision.wrapping_add(1);
        }
        removed
    }

    pub fn commit_remove(&mut self, removed: Vec<(usize, Stroke)>) {
        if removed.is_empty() {
            return;
        }
        self.push(Action::Remove(removed));
    }

    pub fn restore_removed(&mut self, removed: &[(usize, Stroke)]) {
        if removed.is_empty() {
            return;
        }
        for (index, stroke) in removed.iter().rev() {
            let index = (*index).min(self.strokes.len());
            self.strokes.insert(index, stroke.clone());
        }
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn clear(&mut self) {
        if self.strokes.is_empty() {
            return;
        }
        let old = std::mem::take(&mut self.strokes);
        self.push(Action::Clear(old));
    }

    pub fn undo(&mut self) -> bool {
        let Some(action) = self.undo.pop() else {
            return false;
        };
        match &action {
            Action::Add(_) => {
                self.strokes.pop();
            }
            Action::Remove(items) => {
                for (index, stroke) in items.iter().rev() {
                    let index = (*index).min(self.strokes.len());
                    self.strokes.insert(index, stroke.clone());
                }
            }
            Action::Clear(strokes) => {
                self.strokes.clone_from(strokes);
            }
        }
        self.redo.push(action);
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(action) = self.redo.pop() else {
            return false;
        };
        match &action {
            Action::Add(stroke) => self.strokes.push(stroke.clone()),
            Action::Remove(items) => {
                for (index, _) in items {
                    if *index < self.strokes.len() {
                        self.strokes.remove(*index);
                    }
                }
            }
            Action::Clear(_) => self.strokes.clear(),
        }
        self.undo.push(action);
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    fn push(&mut self, action: Action) {
        self.undo.push(action);
        if self.undo.len() > 200 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }
}

#[derive(Serialize, Deserialize)]
struct BoardFile {
    version: u32,
    strokes: Vec<Stroke>,
    camera: FileCamera,
}

#[derive(Serialize, Deserialize)]
struct FileCamera {
    x: f32,
    y: f32,
    zoom: f32,
}

pub struct Loaded {
    pub document: Document,
    pub camera: Camera,
}

pub fn load(path: &Path) -> Result<Loaded, String> {
    let text = std::fs::read_to_string(path).map_err(|err| err.to_string())?;
    let file: BoardFile = serde_json::from_str(&text).map_err(|err| err.to_string())?;
    if file.version != 1 {
        return Err(format!("unsupported board version {}", file.version));
    }
    let mut document = Document::default();
    document.strokes = file.strokes;
    Ok(Loaded {
        document,
        camera: Camera {
            center: Vec2::new(file.camera.x, file.camera.y),
            zoom: file.camera.zoom.clamp(0.2, 8.0),
        },
    })
}

pub fn save(path: &Path, document: &Document, camera: &Camera) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let file = BoardFile {
        version: 1,
        strokes: document.strokes.clone(),
        camera: FileCamera {
            x: camera.center.x,
            y: camera.center.y,
            zoom: camera.zoom,
        },
    };
    let json = serde_json::to_vec_pretty(&file).map_err(|err| err.to_string())?;
    std::fs::write(path, json).map_err(|err| err.to_string())
}

pub fn default_path() -> std::path::PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".local/share"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join("ondeck").join("board.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(y: f32) -> Stroke {
        Stroke {
            points: vec![[0.0, y], [10.0, y]],
            color: [0.0, 0.0, 0.0, 1.0],
            width: 2.0,
        }
    }

    #[test]
    fn undo_redo_add_and_clear() {
        let mut doc = Document::default();
        doc.add_stroke(stroke(0.0));
        doc.add_stroke(stroke(5.0));
        assert!(doc.undo());
        assert_eq!(doc.strokes.len(), 1);
        assert!(doc.redo());
        assert_eq!(doc.strokes.len(), 2);
        doc.clear();
        assert!(doc.strokes.is_empty());
        assert!(doc.undo());
        assert_eq!(doc.strokes.len(), 2);
    }

    #[test]
    fn erase_restores_order() {
        let mut doc = Document::default();
        doc.add_stroke(stroke(0.0));
        doc.add_stroke(stroke(20.0));
        doc.add_stroke(stroke(40.0));
        let removed = doc.extract_hits(&[Vec2::new(5.0, 20.0)], 3.0);
        assert_eq!(removed.len(), 1);
        assert_eq!(doc.strokes.len(), 2);
        doc.restore_removed(&removed);
        assert_eq!(doc.strokes[1].points[0][1], 20.0);
    }
}

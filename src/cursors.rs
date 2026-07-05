use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const CURSOR_FILE_NAME: &str = ".slack-wf-trigger.cursors.json";

pub type ChannelId = String;
pub type LatestTs = String;

#[derive(Debug, Default, Clone)]
pub struct CursorStore {
    path: PathBuf,
    cursors: HashMap<ChannelId, LatestTs>,
}

impl CursorStore {
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join(CURSOR_FILE_NAME);
        let cursors = if path.exists() {
            let bytes = fs::read(&path)
                .with_context(|| format!("failed to read cursor file {}", path.display()))?;
            if bytes.is_empty() {
                HashMap::new()
            } else {
                serde_json::from_slice(&bytes)
                    .with_context(|| format!("cursor file {} is malformed", path.display()))?
            }
        } else {
            HashMap::new()
        };

        Ok(Self { path, cursors })
    }

    pub fn get(&self, channel_id: &str) -> Option<&LatestTs> {
        self.cursors.get(channel_id)
    }

    pub fn set(&mut self, channel_id: ChannelId, ts: LatestTs) {
        self.cursors.insert(channel_id, ts);
    }

    pub fn persist(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create cursor directory {}", parent.display())
            })?;
        }

        let tmp = self.path.with_extension("json.tmp");
        let bytes =
            serde_json::to_vec_pretty(&self.cursors).context("failed to serialise cursor map")?;

        {
            let mut f = fs::File::create(&tmp)
                .with_context(|| format!("failed to create {}", tmp.display()))?;
            f.write_all(&bytes)
                .with_context(|| format!("failed to write {}", tmp.display()))?;
            f.sync_all().ok();
        }

        fs::rename(&tmp, &self.path).with_context(|| {
            format!(
                "failed to rename {} to {}",
                tmp.display(),
                self.path.display()
            )
        })?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, body: &str) {
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn returns_empty_when_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = CursorStore::load(dir.path()).unwrap();
        assert!(store.cursors.is_empty());
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = CursorStore::load(dir.path()).unwrap();
        store.set("C1".into(), "1717600042.000456".into());
        store.set("C2".into(), "1717600100.000789".into());
        store.persist().unwrap();

        assert!(dir.path().join(CURSOR_FILE_NAME).exists());

        let reloaded = CursorStore::load(dir.path()).unwrap();
        assert_eq!(reloaded.get("C1"), Some(&"1717600042.000456".to_string()));
        assert_eq!(reloaded.get("C2"), Some(&"1717600100.000789".to_string()));
    }

    #[test]
    fn writes_atomically_no_tmp_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        let store = CursorStore::load(dir.path()).unwrap();
        store.persist().unwrap();
        assert!(!dir.path().join("cursors.json.tmp").exists());
    }

    #[test]
    fn rejects_malformed_json() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join(CURSOR_FILE_NAME), "not json");
        assert!(CursorStore::load(dir.path()).is_err());
    }

    #[test]
    fn overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        write_file(&dir.path().join(CURSOR_FILE_NAME), r#"{"C1":"old"}"#);
        let mut store = CursorStore::load(dir.path()).unwrap();
        store.set("C2".into(), "new".into());
        store.persist().unwrap();

        let reloaded = CursorStore::load(dir.path()).unwrap();
        assert_eq!(reloaded.get("C1"), Some(&"old".to_string()));
        assert_eq!(reloaded.get("C2"), Some(&"new".to_string()));
    }
}

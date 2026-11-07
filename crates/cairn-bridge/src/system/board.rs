//! Home widget layout: saved to and loaded from `<data dir>/widgets.json`.
//! The UI decides what the layout contains. This module stores it unchanged.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::{BoardLayout, Bridge, Model, ToastKind};

const FILE_NAME: &str = "widgets.json";

impl Bridge {
    /// Saves the layout. `m.board` changes at once and the file is written in the background.
    /// Only the latest of several queued writes is kept. Without a data dir (startup not done)
    /// only memory changes. `shutdown` waits for the last save. A failed write is reported.
    pub fn set_board(&self, m: &mut Model, layout: BoardLayout) {
        if m.board.as_ref() == Some(&layout) {
            return;
        }
        m.board = Some(layout.clone());
        if let Some(dir) = m.data_dir.clone() {
            self.queue_board(dir, Some(layout));
        }
    }

    /// Restores the default layout. `m.board` becomes `None` at once and the file is deleted in
    /// the background. The delete is ordered after queued saves, so an old save cannot write it back.
    pub fn reset_board(&self, m: &mut Model) {
        m.board = None;
        if let Some(dir) = m.data_dir.clone() {
            self.queue_board(dir, None);
        }
    }

    pub fn refresh_hitokoto(&self, m: &mut Model) {
        let _ = m;
        let b = self.clone();
        self.inner.rt.spawn(async move {
            if let Some(hitokoto) = crate::fetch_hitokoto().await {
                b.post(move |_, m| m.hitokoto = Some(hitokoto));
            }
        });
    }

    /// Queues a write (`Some`) or a delete (`None`). Each call takes a new generation.
    /// A queued job that is no longer the newest skips itself.
    /// Writes and deletes share a lock and generation, so an older save cannot undo a delete.
    fn queue_board(&self, dir: PathBuf, layout: Option<BoardLayout>) {
        let generation = self.inner.board_gen.fetch_add(1, Ordering::SeqCst) + 1;
        let inner = self.inner.clone();
        let b = self.clone();
        let save = self.inner.rt.spawn(async move {
            let _guard = inner.board_lock.lock().await;
            if inner.board_gen.load(Ordering::SeqCst) != generation {
                return;
            }
            let result = match &layout {
                Some(layout) => write(&dir, layout).await.map_err(|e| format!("保存主页布局失败:{e}")),
                None => remove(&dir).await.map_err(|e| format!("恢复默认布局失败:{e}")),
            };
            if let Err(msg) = result {
                b.post(move |b, m| b.toast(m, ToastKind::Error, msg));
            }
        });
        *self.inner.board_save.lock().unwrap() = Some(save);
    }
}

/// Reads the layout at startup. A missing or malformed file gives `None`,
/// so the UI uses the default layout.
pub(crate) async fn load(dir: &Path) -> Option<BoardLayout> {
    let bytes = tokio::fs::read(dir.join(FILE_NAME)).await.ok()?;
    parse(&bytes)
}

/// Parses `widgets.json`. Invalid JSON gives `None`. Unknown fields are ignored, and a missing
/// `options` counts as an empty map.
fn parse(bytes: &[u8]) -> Option<BoardLayout> {
    serde_json::from_slice(bytes).ok()
}

/// Writes a temp file and renames it over the target, so a power loss mid-write cannot leave a
/// broken file.
async fn write(dir: &Path, layout: &BoardLayout) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(layout).map_err(io::Error::other)?;
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    tokio::fs::write(&tmp, &json).await?;
    tokio::fs::rename(&tmp, dir.join(FILE_NAME)).await
}

async fn remove(dir: &Path) -> io::Result<()> {
    match tokio::fs::remove_file(dir.join(FILE_NAME)).await {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::BoardItem;

    fn sample() -> BoardLayout {
        BoardLayout {
            version: 1,
            hide_empty_hint: true,
            items: vec![
                BoardItem {
                    id: "clock-1".into(),
                    kind: "clock".into(),
                    x: 0,
                    y: 0,
                    w: 2,
                    h: 2,
                    options: BTreeMap::from([("face".to_string(), "analog".to_string())]),
                },
                BoardItem { id: "heatmap-1".into(), kind: "heatmap".into(), x: 2, y: 0, w: 4, h: 2, options: BTreeMap::new() },
            ],
        }
    }

    #[test]
    fn parses_a_valid_layout() {
        let json = br#"{"version":1,"items":[{"id":"clock-1","kind":"clock","x":0,"y":1,"w":2,"h":3,"options":{"face":"analog"}}]}"#;
        let layout = parse(json).expect("合法的 JSON 应该能解析");
        assert_eq!(layout.version, 1);
        assert!(!layout.hide_empty_hint);
        assert_eq!(layout.items.len(), 1);
        let item = &layout.items[0];
        assert_eq!((item.id.as_str(), item.kind.as_str()), ("clock-1", "clock"));
        assert_eq!((item.x, item.y, item.w, item.h), (0, 1, 2, 3));
        assert_eq!(item.options.get("face").map(String::as_str), Some("analog"));
    }

    #[test]
    fn missing_options_means_an_empty_table() {
        let json = br#"{"version":1,"items":[{"id":"heatmap-1","kind":"heatmap","x":0,"y":0,"w":4,"h":2}]}"#;
        let layout = parse(json).expect("缺少 options 也应该能解析");
        assert!(layout.items[0].options.is_empty());
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let json = br#"{"version":1,"future":true,"items":[{"id":"a","kind":"clock","x":0,"y":0,"w":1,"h":1,"color":"red"}]}"#;
        let layout = parse(json).expect("多出的未知字段应该被忽略");
        assert_eq!(layout.items.len(), 1);
        assert_eq!(layout.items[0].id, "a");
    }

    #[test]
    fn invalid_json_is_none() {
        assert_eq!(parse(b"{ not json"), None);
        assert_eq!(parse(b""), None);
    }

    #[test]
    fn serialize_then_parse_gives_an_equal_layout() {
        let layout = sample();
        let json = serde_json::to_vec_pretty(&layout).unwrap();
        assert_eq!(parse(&json), Some(layout));
    }

    #[test]
    fn empty_options_are_not_serialized() {
        let value = serde_json::to_value(sample()).unwrap();
        assert!(value["items"][0].get("options").is_some());
        assert!(value["items"][1].get("options").is_none());
    }

    #[tokio::test]
    async fn write_load_and_remove_on_disk() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let dir: PathBuf = std::env::temp_dir().join(format!("cairn-board-test-{}-{nanos}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        write(&dir, &sample()).await.unwrap();
        assert!(!dir.join("widgets.json.tmp").exists());
        assert_eq!(load(&dir).await, Some(sample()));
        remove(&dir).await.unwrap();
        assert_eq!(load(&dir).await, None);
        // The file is already gone. Deleting it again is not an error.
        remove(&dir).await.unwrap();
        tokio::fs::remove_dir_all(&dir).await.unwrap();
    }
}

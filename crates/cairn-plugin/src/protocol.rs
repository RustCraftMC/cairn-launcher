//! Messages between the launcher and the plugin host, sent over the host's stdin and stdout.
//! Each frame is a little-endian `u32` length followed by that many bytes of JSON.

use std::io::{self, Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::manifest::Permission;

/// Largest accepted frame. Larger frames mean a broken peer; the connection is dropped.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// Everything the host needs to start one plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSpec {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Package folder; module imports cannot leave it.
    pub dir: PathBuf,
    /// Entry module relative to `dir`.
    pub main: String,
    /// The plugin's own folder for files and storage.
    pub data_dir: PathBuf,
    pub permissions: Vec<Permission>,
    pub commands: Vec<String>,
    pub widgets: Vec<String>,
    /// Ids of the panels declared in plugin.json.
    #[serde(default)]
    pub panels: Vec<String>,
    /// Current values of the plugin's settings.
    #[serde(default)]
    pub settings: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Target {
    Command {
        id: String,
    },
    /// The plugin's `cairn.launch.onBeforeLaunch` handler.
    BeforeLaunch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToHost {
    Load {
        spec: LoadSpec,
    },
    Unload {
        plugin: String,
    },
    /// An event for one plugin, or for every plugin when `plugin` is `None`.
    Event {
        plugin: Option<String>,
        name: String,
        data: Value,
    },
    /// Calls into a plugin; answered with [`FromHost::Done`] carrying the same `call`.
    Invoke {
        call: u64,
        plugin: String,
        target: Target,
        data: Value,
    },
    /// Answer to a [`FromHost::Request`].
    Reply {
        request: u64,
        result: Result<Value, String>,
    },
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromHost {
    Ready,
    /// `activate()` finished. `launch_hook` tells whether the plugin registered a launch handler.
    Activated {
        plugin: String,
        launch_hook: bool,
        millis: f64,
    },
    /// Loading or activation failed, or the plugin was stopped after a fatal error. It no longer runs.
    Failed {
        plugin: String,
        error: String,
    },
    Stopped {
        plugin: String,
    },
    Log {
        plugin: String,
        level: LogLevel,
        message: String,
    },
    /// New data and/or view for a widget. Only the latest one per widget matters.
    /// A plugin panel is updated through the same message: its `widget` is `panel:<panel id>`.
    Widget {
        plugin: String,
        widget: String,
        data: Option<serde_json::Map<String, Value>>,
        view: Option<Value>,
    },
    Done {
        call: u64,
        result: Result<Value, String>,
    },
    /// A launcher operation that needs an answer ([`ToHost::Reply`]).
    Request {
        request: u64,
        plugin: String,
        method: String,
        params: Value,
    },
    /// A launcher operation without an answer, such as showing a toast.
    Notify {
        plugin: String,
        method: String,
        params: Value,
    },
}

pub fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T) -> io::Result<()> {
    let body = serde_json::to_vec(msg).map_err(io::Error::other)?;
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "frame too large"));
    }
    w.write_all(&(body.len() as u32).to_le_bytes())?;
    w.write_all(&body)?;
    w.flush()
}

/// Reads one frame. `Ok(None)` means the peer closed the stream cleanly between frames.
pub fn read_frame<R: Read, T: for<'de> Deserialize<'de>>(r: &mut R) -> io::Result<Option<T>> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("bad frame length {len}")));
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body).map(Some).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_reject_garbage() {
        let mut buf = Vec::new();
        let msg = ToHost::Invoke { call: 7, plugin: "demo".into(), target: Target::BeforeLaunch, data: serde_json::json!({"a": 1}) };
        write_frame(&mut buf, &msg).unwrap();
        write_frame(&mut buf, &ToHost::Shutdown).unwrap();
        let mut r = buf.as_slice();
        assert_eq!(read_frame::<_, ToHost>(&mut r).unwrap(), Some(msg));
        assert_eq!(read_frame::<_, ToHost>(&mut r).unwrap(), Some(ToHost::Shutdown));
        assert_eq!(read_frame::<_, ToHost>(&mut r).unwrap(), None);
        for bad in [&[0u8, 0, 0, 0][..], &[255, 255, 255, 255], &[4, 0, 0, 0, b'{', b'}', b'}', b'}'], &[9, 0, 0, 0, b'{']] {
            assert!(read_frame::<_, ToHost>(&mut &bad[..]).is_err());
        }
    }
}

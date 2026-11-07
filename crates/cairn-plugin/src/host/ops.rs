//! Native operations behind the SDK: files and storage run synchronously on the plugin's thread;
//! HTTP and processes run on the host's shared Tokio runtime.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::manifest::{self, Permission};
use crate::protocol::LoadSpec;

const MAX_READ: u64 = 64 * 1024 * 1024;
const MAX_STORAGE: usize = 16 * 1024 * 1024;
const MAX_RESPONSE: usize = 32 * 1024 * 1024;
const MAX_OUTPUT: usize = 8 * 1024 * 1024;

pub(super) type OpResult = Result<Value, String>;

fn args<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| format!("Invalid arguments: {e}"))
}

fn granted(spec: &LoadSpec, permission: Permission) -> Result<(), String> {
    if spec.permissions.contains(&permission) { Ok(()) } else { Err(format!("Permission '{}' was not granted", permission.key())) }
}

/// Removes `.` and resolves `..` without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Resolves a path given by the plugin. Relative paths are inside the plugin's data folder.
/// Without the `fs` permission the plugin may only use that folder, and read its own package.
pub(super) fn resolve(spec: &LoadSpec, path: &str, write: bool) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("Path is empty".into());
    }
    let full = normalize(&spec.data_dir.join(path));
    if spec.permissions.contains(&Permission::Fs) {
        return Ok(full);
    }
    let data = normalize(&spec.data_dir);
    if full.starts_with(&data) || (!write && full.starts_with(normalize(&spec.dir))) {
        Ok(full)
    } else {
        Err(format!("'{path}' is outside the plugin folder; the 'fs' permission is needed"))
    }
}

fn io_err(path: &Path) -> impl Fn(io::Error) -> String + '_ {
    move |e| format!("{}: {e}", path.display())
}

fn unix_millis(time: io::Result<SystemTime>) -> Value {
    time.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(Value::Null, |d| json!(d.as_millis() as u64))
}

pub(super) fn fs_call(spec: &LoadSpec, method: &str, value: Value) -> OpResult {
    #[derive(Deserialize)]
    struct PathArgs {
        path: String,
        #[serde(default)]
        text: String,
        #[serde(default)]
        data: String,
        #[serde(default)]
        recursive: bool,
    }
    #[derive(Deserialize)]
    struct Move {
        from: String,
        to: String,
    }
    if matches!(method, "fs.rename" | "fs.copy") {
        let Move { from, to } = args(value)?;
        let (from, to) = (resolve(spec, &from, method == "fs.rename")?, resolve(spec, &to, true)?);
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(io_err(parent))?;
        }
        if method == "fs.rename" {
            fs::rename(&from, &to).map_err(io_err(&from))?;
        } else {
            fs::copy(&from, &to).map_err(io_err(&from))?;
        }
        return Ok(Value::Null);
    }
    let a: PathArgs = args(value)?;
    let write = matches!(method, "fs.writeText" | "fs.writeBase64" | "fs.appendText" | "fs.mkdir" | "fs.remove");
    let path = resolve(spec, &a.path, write)?;
    let ensure_parent = |path: &Path| match path.parent() {
        Some(parent) => fs::create_dir_all(parent).map_err(io_err(parent)),
        None => Ok(()),
    };
    match method {
        "fs.resolve" => Ok(json!(path.to_string_lossy())),
        "fs.exists" => Ok(json!(path.exists())),
        "fs.stat" => {
            let meta = fs::metadata(&path).map_err(io_err(&path))?;
            Ok(json!({"dir": meta.is_dir(), "file": meta.is_file(), "size": meta.len(), "modified": unix_millis(meta.modified())}))
        }
        "fs.list" => {
            let mut out = Vec::new();
            for entry in fs::read_dir(&path).map_err(io_err(&path))? {
                let entry = entry.map_err(io_err(&path))?;
                let meta = entry.metadata().ok();
                out.push(json!({
                    "name": entry.file_name().to_string_lossy(),
                    "dir": meta.as_ref().is_some_and(|m| m.is_dir()),
                    "size": meta.as_ref().map_or(0, |m| m.len()),
                    "modified": meta.map_or(Value::Null, |m| unix_millis(m.modified())),
                }));
            }
            Ok(Value::Array(out))
        }
        "fs.readText" => {
            let bytes = manifest::read_limited(&path, MAX_READ).map_err(io_err(&path))?;
            Ok(json!(String::from_utf8_lossy(&bytes)))
        }
        "fs.readBase64" => Ok(json!(base64::encode(&manifest::read_limited(&path, MAX_READ).map_err(io_err(&path))?))),
        "fs.writeText" => {
            ensure_parent(&path)?;
            manifest::write_atomic(&path, a.text.as_bytes()).map_err(io_err(&path))?;
            Ok(Value::Null)
        }
        "fs.writeBase64" => {
            ensure_parent(&path)?;
            manifest::write_atomic(&path, &base64::decode(&a.data)?).map_err(io_err(&path))?;
            Ok(Value::Null)
        }
        "fs.appendText" => {
            ensure_parent(&path)?;
            fs::OpenOptions::new().create(true).append(true).open(&path).and_then(|mut f| f.write_all(a.text.as_bytes())).map_err(io_err(&path))?;
            Ok(Value::Null)
        }
        "fs.mkdir" => {
            fs::create_dir_all(&path).map_err(io_err(&path))?;
            Ok(Value::Null)
        }
        "fs.remove" => {
            if normalize(&path) == normalize(&spec.data_dir) {
                return Err("The plugin data folder itself cannot be removed".into());
            }
            let result = match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_dir() && a.recursive => fs::remove_dir_all(&path),
                Ok(meta) if meta.is_dir() => fs::remove_dir(&path),
                Ok(_) => fs::remove_file(&path),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e),
            };
            result.map_err(io_err(&path))?;
            Ok(Value::Null)
        }
        _ => Err(format!("Unknown operation '{method}'")),
    }
}

/// The plugin's key-value store, kept in memory and written to `storage.json` in batches.
pub(super) struct Storage {
    path: PathBuf,
    map: Option<Map<String, Value>>,
    pub dirty: bool,
}

impl Storage {
    pub fn new(data_dir: &Path) -> Self {
        Self { path: data_dir.join("storage.json"), map: None, dirty: false }
    }

    /// Loaded on first use, so plugins that never use storage never read the file.
    fn map(&mut self) -> Result<&mut Map<String, Value>, String> {
        if self.map.is_none() {
            let map = match manifest::read_limited(&self.path, MAX_STORAGE as u64) {
                Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", self.path.display()))?,
                Err(e) if e.kind() == io::ErrorKind::NotFound => Map::new(),
                Err(e) => return Err(format!("{}: {e}", self.path.display())),
            };
            self.map = Some(map);
        }
        Ok(self.map.as_mut().unwrap())
    }

    pub fn call(&mut self, method: &str, value: Value) -> OpResult {
        #[derive(Deserialize)]
        struct Key {
            #[serde(default)]
            key: String,
            #[serde(default)]
            value: Value,
        }
        let a: Key = args(value)?;
        let map = self.map()?;
        let changed = match method {
            "storage.get" => return Ok(map.get(&a.key).cloned().unwrap_or(Value::Null)),
            "storage.keys" => return Ok(json!(map.keys().collect::<Vec<_>>())),
            "storage.set" => {
                if a.key.is_empty() || a.key.len() > 256 {
                    return Err("Storage keys must be 1-256 bytes".into());
                }
                map.insert(a.key, a.value);
                if serde_json::to_vec(map).map_or(0, |v| v.len()) > MAX_STORAGE {
                    return Err("Plugin storage is limited to 16 MiB".into());
                }
                true
            }
            "storage.remove" => map.remove(&a.key).is_some(),
            "storage.clear" => {
                let had = !map.is_empty();
                map.clear();
                had
            }
            _ => return Err(format!("Unknown operation '{method}'")),
        };
        self.dirty |= changed;
        Ok(Value::Null)
    }

    pub fn flush(&mut self) -> Result<(), String> {
        let (Some(map), true) = (&self.map, self.dirty) else { return Ok(()) };
        let bytes = serde_json::to_vec(map).map_err(|e| e.to_string())?;
        manifest::write_atomic(&self.path, &bytes).map_err(|e| format!("{}: {e}", self.path.display()))?;
        self.dirty = false;
        Ok(())
    }
}

#[derive(Deserialize)]
struct HttpArgs {
    url: String,
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default, rename = "timeoutMs")]
    timeout_ms: Option<u64>,
    #[serde(default)]
    path: Option<String>,
}

impl HttpArgs {
    fn build(&self, client: &reqwest::Client) -> Result<reqwest::RequestBuilder, String> {
        let url = reqwest::Url::parse(&self.url).map_err(|e| format!("Invalid URL '{}': {e}", self.url))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("Only http and https URLs are supported".into());
        }
        let method = reqwest::Method::from_bytes(self.method.as_deref().unwrap_or("GET").to_ascii_uppercase().as_bytes()).map_err(|e| e.to_string())?;
        let mut request = client.request(method, url).timeout(Duration::from_millis(self.timeout_ms.unwrap_or(30_000).clamp(100, 600_000)));
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(body) = &self.body {
            request = request.body(body.clone());
        }
        Ok(request)
    }
}

pub(super) async fn http(client: reqwest::Client, spec: LoadSpec, method: String, value: Value) -> OpResult {
    granted(&spec, Permission::Network)?;
    let a: HttpArgs = args(value)?;
    let mut response = a.build(&client)?.send().await.map_err(|e| e.to_string())?;
    let status = response.status().as_u16();
    let url = response.url().to_string();
    if method == "http.download" {
        let path = resolve(&spec, a.path.as_deref().unwrap_or_default(), true)?;
        if !response.status().is_success() {
            return Err(format!("HTTP {status} from {url}"));
        }
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io_err(parent))?;
        }
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".part");
        let tmp = PathBuf::from(tmp);
        let mut file = tokio::fs::File::create(&tmp).await.map_err(io_err(&tmp))?;
        let mut size = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
            size += chunk.len() as u64;
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await.map_err(io_err(&tmp))?;
        }
        tokio::io::AsyncWriteExt::flush(&mut file).await.map_err(io_err(&tmp))?;
        drop(file);
        tokio::fs::rename(&tmp, &path).await.map_err(io_err(&path))?;
        return Ok(json!({"status": status, "path": path.to_string_lossy(), "size": size}));
    }
    let headers: Map<String, Value> = response.headers().iter().filter_map(|(k, v)| Some((k.as_str().to_string(), json!(v.to_str().ok()?)))).collect();
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if body.len() + chunk.len() > MAX_RESPONSE {
            return Err("Response exceeds 32 MiB; use http.download".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(json!({"status": status, "url": url, "headers": headers, "body": String::from_utf8_lossy(&body)}))
}

#[derive(Deserialize)]
struct ProcessArgs {
    #[serde(default)]
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default, rename = "timeoutMs")]
    timeout_ms: Option<u64>,
    #[serde(default)]
    stdin: Option<String>,
    #[serde(default)]
    target: String,
}

fn command(spec: &LoadSpec, a: &ProcessArgs) -> Result<tokio::process::Command, String> {
    if a.program.is_empty() {
        return Err("Program is empty".into());
    }
    let mut cmd = tokio::process::Command::new(&a.program);
    cmd.args(&a.args).envs(&a.env).current_dir(match &a.cwd {
        Some(cwd) => resolve(spec, cwd, false)?,
        None => spec.data_dir.clone(),
    });
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    Ok(cmd)
}

pub(super) async fn process(spec: LoadSpec, method: String, value: Value) -> OpResult {
    granted(&spec, Permission::Process)?;
    let a: ProcessArgs = args(value)?;
    match method.as_str() {
        "process.run" => {
            use std::process::Stdio;
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut child = command(&spec, &a)?
                .stdin(if a.stdin.is_some() { Stdio::piped() } else { Stdio::null() })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("{}: {e}", a.program))?;
            if let (Some(input), Some(mut stdin)) = (a.stdin, child.stdin.take()) {
                tokio::spawn(async move {
                    let _ = stdin.write_all(input.as_bytes()).await;
                });
            }
            let read = |pipe: Option<Box<dyn tokio::io::AsyncRead + Unpin + Send>>| async move {
                let mut out = Vec::new();
                if let Some(pipe) = pipe {
                    let _ = pipe.take(MAX_OUTPUT as u64).read_to_end(&mut out).await;
                }
                String::from_utf8_lossy(&out).into_owned()
            };
            let stdout = child.stdout.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>);
            let stderr = child.stderr.take().map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>);
            let timeout = Duration::from_millis(a.timeout_ms.unwrap_or(60_000).clamp(100, 3_600_000));
            let run = async { tokio::join!(child.wait(), read(stdout), read(stderr)) };
            let (status, stdout, stderr) = tokio::time::timeout(timeout, run).await.map_err(|_| format!("{} timed out after {timeout:?}", a.program))?;
            let status = status.map_err(|e| e.to_string())?;
            Ok(json!({"code": status.code(), "stdout": stdout, "stderr": stderr}))
        }
        "process.spawn" => {
            let child = command(&spec, &a)?.spawn().map_err(|e| format!("{}: {e}", a.program))?;
            Ok(json!({"pid": child.id()}))
        }
        "process.open" => {
            if a.target.is_empty() {
                return Err("Nothing to open".into());
            }
            #[cfg(windows)]
            let mut cmd = {
                let mut cmd = tokio::process::Command::new("rundll32.exe");
                cmd.args(["url.dll,FileProtocolHandler", &a.target]);
                cmd
            };
            #[cfg(target_os = "macos")]
            let mut cmd = {
                let mut cmd = tokio::process::Command::new("open");
                cmd.arg(&a.target);
                cmd
            };
            #[cfg(all(unix, not(target_os = "macos")))]
            let mut cmd = {
                let mut cmd = tokio::process::Command::new("xdg-open");
                cmd.arg(&a.target);
                cmd
            };
            cmd.spawn().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        _ => Err(format!("Unknown operation '{method}'")),
    }
}

mod base64 {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
            for i in 0..4 {
                out.push(if i <= chunk.len() { TABLE[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
            }
        }
        out
    }

    pub fn decode(text: &str) -> Result<Vec<u8>, String> {
        let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        let digits = digits.strip_suffix(b"==").or_else(|| digits.strip_suffix(b"=")).unwrap_or(&digits);
        let mut out = Vec::with_capacity(digits.len() * 3 / 4);
        let (mut acc, mut bits) = (0u32, 0);
        for &b in digits {
            let v = TABLE.iter().position(|&t| t == b).ok_or("Invalid base64")? as u32;
            acc = acc << 6 | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(dir: &Path, permissions: Vec<Permission>) -> LoadSpec {
        LoadSpec {
            id: "demo".into(),
            name: "Demo".into(),
            version: "1".into(),
            dir: dir.join("plugins/demo"),
            main: "main.js".into(),
            data_dir: dir.join("plugin-data/demo"),
            permissions,
            commands: vec![],
            widgets: vec![],
            panels: vec![],
            settings: Default::default(),
        }
    }

    #[test]
    fn paths_stay_in_the_plugin_folder_without_fs() {
        let root = Path::new("C:/cairn");
        let sandboxed = spec(root, vec![]);
        assert_eq!(resolve(&sandboxed, "notes/a.txt", true).unwrap(), root.join("plugin-data/demo/notes/a.txt"));
        assert!(resolve(&sandboxed, "../other/a.txt", false).is_err());
        assert!(resolve(&sandboxed, "C:/Windows/win.ini", false).is_err());
        assert!(resolve(&sandboxed, "../../plugins/demo/main.js", false).is_ok());
        assert!(resolve(&sandboxed, "../../plugins/demo/main.js", true).is_err());
        assert!(resolve(&spec(root, vec![Permission::Fs]), "C:/Windows/win.ini", false).is_ok());
    }

    #[test]
    fn base64_round_trip() {
        for len in 0..20 {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(base64::decode(&base64::encode(&data)).unwrap(), data);
        }
        assert_eq!(base64::encode(b"Cairn"), "Q2Fpcm4=");
        assert!(base64::decode("@@").is_err());
    }
}

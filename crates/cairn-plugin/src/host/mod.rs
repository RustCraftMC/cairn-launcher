//! The plugin host process. The launcher starts its own executable with [`HOST_FLAG`]; that process
//! runs every enabled script plugin, each on its own thread with its own QuickJS runtime, and talks
//! to the launcher over stdin and stdout. A crashing or hanging plugin cannot take the launcher down.

mod ops;
mod runtime;

use std::collections::HashMap;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::protocol::{FromHost, ToHost, read_frame, write_frame};
use runtime::Msg;

/// Command-line flag that turns the launcher executable into the plugin host.
pub const HOST_FLAG: &str = "--internal-plugin-host";
const PLUGIN_STACK: usize = 8 * 1024 * 1024;
const STOP_GRACE: Duration = Duration::from_millis(1500);

/// Resources shared by all plugins in the host. The Tokio runtime and HTTP client are created on
/// first use, so plugins that never do I/O cost nothing extra.
pub(crate) struct Shared {
    runtime: OnceLock<tokio::runtime::Runtime>,
    http: OnceLock<reqwest::Client>,
    next_request: AtomicU64,
    requests: Mutex<HashMap<u64, Sender<Msg>>>,
}

impl Shared {
    fn runtime(&self) -> &tokio::runtime::Runtime {
        self.runtime.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("plugin-io").enable_all().build().expect("plugin I/O runtime")
        })
    }

    fn http(&self) -> reqwest::Client {
        self.http
            .get_or_init(|| reqwest::Client::builder().user_agent(concat!("CairnLauncher-plugin/", env!("CARGO_PKG_VERSION"))).build().unwrap_or_default())
            .clone()
    }

    /// Allocates a request id whose reply is routed to `mailbox`.
    fn request(&self, mailbox: Sender<Msg>) -> u64 {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed) + 1;
        self.requests.lock().unwrap().insert(id, mailbox);
        id
    }
}

struct Running {
    mailbox: Sender<Msg>,
    thread: JoinHandle<()>,
}

/// Entry point of the host process. Returns when the launcher asks it to stop or goes away.
pub fn run() {
    serve(io::stdin(), io::stdout());
}

/// Serves the protocol on any pair of streams. Tests use in-process pipes.
pub fn serve<R: Read, W: Write + Send + 'static>(input: R, output: W) {
    let (out, outgoing) = mpsc::channel::<FromHost>();
    let writer = std::thread::Builder::new()
        .name("plugin-host-out".into())
        .spawn(move || {
            let mut output = BufWriter::new(output);
            for msg in outgoing {
                if write_frame(&mut output, &msg).is_err() {
                    break;
                }
            }
        })
        .expect("plugin host writer");
    let shared = Arc::new(Shared { runtime: OnceLock::new(), http: OnceLock::new(), next_request: AtomicU64::new(0), requests: Mutex::default() });
    let mut plugins: HashMap<String, Running> = HashMap::new();
    let mut input = BufReader::new(input);
    let _ = out.send(FromHost::Ready);
    while let Ok(Some(msg)) = read_frame::<_, ToHost>(&mut input) {
        plugins.retain(|_, p| !p.thread.is_finished());
        match msg {
            ToHost::Load { spec } => {
                if let Some(old) = plugins.remove(&spec.id) {
                    let _ = old.mailbox.send(Msg::Stop);
                    let _ = old.thread.join();
                }
                let (mailbox, rx) = mpsc::channel();
                let (id, own, sender, shared) = (spec.id.clone(), mailbox.clone(), out.clone(), shared.clone());
                match std::thread::Builder::new()
                    .name(format!("plugin-{id}"))
                    .stack_size(PLUGIN_STACK)
                    .spawn(move || runtime::run(spec, rx, own, sender, shared))
                {
                    Ok(thread) => {
                        plugins.insert(id, Running { mailbox, thread });
                    }
                    Err(e) => {
                        let _ = out.send(FromHost::Failed { plugin: id, error: e.to_string() });
                    }
                }
            }
            ToHost::Unload { plugin } => match plugins.remove(&plugin) {
                Some(running) => {
                    let _ = running.mailbox.send(Msg::Stop);
                }
                None => {
                    let _ = out.send(FromHost::Stopped { plugin });
                }
            },
            ToHost::Event { plugin, name, data } => {
                let data: Arc<str> = data.to_string().into();
                for (id, running) in &plugins {
                    if plugin.as_ref().is_none_or(|p| p == id) {
                        let _ = running.mailbox.send(Msg::Event { name: name.clone(), data: data.clone() });
                    }
                }
            }
            ToHost::Invoke { call, plugin, target, data } => match plugins.get(&plugin) {
                Some(running) if running.mailbox.send(Msg::Invoke { call, target, data: data.to_string() }).is_ok() => {}
                _ => {
                    let _ = out.send(FromHost::Done { call, result: Err(format!("Plugin '{plugin}' is not running")) });
                }
            },
            ToHost::Reply { request, result } => {
                if let Some(mailbox) = shared.requests.lock().unwrap().remove(&request) {
                    let _ = mailbox.send(Msg::Reply { request, result });
                }
            }
            ToHost::Shutdown => break,
        }
    }
    // Let plugins run deactivate() and save storage, but do not wait forever for one that hangs.
    let stop_by = Instant::now() + STOP_GRACE;
    for running in plugins.values() {
        let _ = running.mailbox.send(Msg::Stop);
    }
    for (_, running) in plugins {
        while !running.thread.is_finished() && Instant::now() < stop_by {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    drop(out);
    while !writer.is_finished() && Instant::now() < stop_by {
        std::thread::sleep(Duration::from_millis(5));
    }
}

//! The launcher's side of the plugin host: starts the host process, sends it messages, matches call
//! results to their callers and reports everything else to a [`Handler`].

use std::collections::HashMap;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::oneshot;

use crate::host::HOST_FLAG;
use crate::protocol::{FromHost, LoadSpec, Target, ToHost, read_frame, write_frame};

/// Receives host messages on the supervisor's reader thread. Implementations should hand work off
/// quickly instead of blocking.
pub trait Handler: Send + Sync + 'static {
    /// Every message except call results, which go to [`Supervisor::invoke`].
    fn message(&self, msg: FromHost);
    /// The host process ended without being asked to.
    fn exited(&self, reason: String);
}

struct Inner {
    tx: Mutex<Option<mpsc::Sender<ToHost>>>,
    calls: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    next_call: AtomicU64,
    child: Mutex<Option<Child>>,
    alive: AtomicBool,
    stopping: AtomicBool,
}

pub struct Supervisor {
    inner: Arc<Inner>,
}

impl Supervisor {
    /// Starts `program` (normally the launcher's own executable) as the plugin host.
    pub fn spawn(program: &Path, handler: Arc<dyn Handler>) -> io::Result<Self> {
        let mut cmd = Command::new(program);
        cmd.arg(HOST_FLAG).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);
        let mut child = cmd.spawn()?;
        let (stdin, stdout) = (child.stdin.take().expect("piped stdin"), child.stdout.take().expect("piped stdout"));
        let supervisor = Self::connect(stdout, stdin, handler);
        *supervisor.inner.child.lock().unwrap() = Some(child);
        Ok(supervisor)
    }

    /// Talks to a host over existing streams: `input` is the host's output and `output` its input.
    pub fn connect<R: Read + Send + 'static, W: Write + Send + 'static>(input: R, output: W, handler: Arc<dyn Handler>) -> Self {
        let (tx, rx) = mpsc::channel::<ToHost>();
        let inner = Arc::new(Inner {
            tx: Mutex::new(Some(tx)),
            calls: Mutex::default(),
            next_call: AtomicU64::new(0),
            child: Mutex::new(None),
            alive: AtomicBool::new(true),
            stopping: AtomicBool::new(false),
        });
        // Writes happen on their own thread so callers on the UI or Tokio threads never block on the pipe.
        std::thread::Builder::new()
            .name("plugin-supervisor-out".into())
            .spawn(move || {
                let mut output = BufWriter::new(output);
                for msg in rx {
                    if write_frame(&mut output, &msg).is_err() {
                        break;
                    }
                }
            })
            .expect("plugin supervisor writer");
        let reader = inner.clone();
        std::thread::Builder::new()
            .name("plugin-supervisor-in".into())
            .spawn(move || {
                let mut input = BufReader::new(input);
                let reason = loop {
                    match read_frame::<_, FromHost>(&mut input) {
                        Ok(Some(FromHost::Done { call, result })) => {
                            if let Some(waiter) = reader.calls.lock().unwrap().remove(&call) {
                                let _ = waiter.send(result);
                            }
                        }
                        Ok(Some(msg)) => handler.message(msg),
                        Ok(None) => break "Plugin host exited".to_string(),
                        Err(e) => break format!("Plugin host connection failed: {e}"),
                    }
                };
                reader.alive.store(false, Ordering::SeqCst);
                for (_, waiter) in reader.calls.lock().unwrap().drain() {
                    let _ = waiter.send(Err(reason.clone()));
                }
                if !reader.stopping.load(Ordering::SeqCst) {
                    handler.exited(reason);
                }
            })
            .expect("plugin supervisor reader");
        Self { inner }
    }

    /// Process id of the host, when it was started with [`Supervisor::spawn`].
    pub fn pid(&self) -> Option<u32> {
        self.inner.child.lock().unwrap().as_ref().map(Child::id)
    }

    pub fn alive(&self) -> bool {
        self.inner.alive.load(Ordering::SeqCst)
    }

    pub fn send(&self, msg: ToHost) -> bool {
        self.alive() && self.inner.tx.lock().unwrap().as_ref().is_some_and(|tx| tx.send(msg).is_ok())
    }

    pub fn load(&self, spec: LoadSpec) -> bool {
        self.send(ToHost::Load { spec })
    }

    pub fn unload(&self, plugin: &str) -> bool {
        self.send(ToHost::Unload { plugin: plugin.into() })
    }

    /// Sends an event to one plugin, or to all of them.
    pub fn event(&self, plugin: Option<&str>, name: &str, data: Value) -> bool {
        self.send(ToHost::Event { plugin: plugin.map(Into::into), name: name.into(), data })
    }

    pub fn reply(&self, request: u64, result: Result<Value, String>) -> bool {
        self.send(ToHost::Reply { request, result })
    }

    /// Calls a plugin's command or launch handler and waits for its result.
    pub async fn invoke(&self, plugin: &str, target: Target, data: Value, timeout: Duration) -> Result<Value, String> {
        let started = self.start_call(plugin, target, data);
        self.finish_call(plugin, started, tokio::time::Instant::now() + timeout, timeout).await
    }

    /// Calls the same target in several plugins at once and returns each result, in order.
    pub async fn invoke_all(&self, plugins: &[String], target: Target, data: Value, timeout: Duration) -> Vec<Result<Value, String>> {
        let deadline = tokio::time::Instant::now() + timeout;
        let started: Vec<_> = plugins.iter().map(|plugin| self.start_call(plugin, target.clone(), data.clone())).collect();
        let mut results = Vec::with_capacity(plugins.len());
        for (plugin, started) in plugins.iter().zip(started) {
            results.push(self.finish_call(plugin, started, deadline, timeout).await);
        }
        results
    }

    fn start_call(&self, plugin: &str, target: Target, data: Value) -> Result<(u64, oneshot::Receiver<Result<Value, String>>), String> {
        let call = self.inner.next_call.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.inner.calls.lock().unwrap().insert(call, tx);
        if !self.send(ToHost::Invoke { call, plugin: plugin.into(), target, data }) {
            self.inner.calls.lock().unwrap().remove(&call);
            return Err("Plugin host is not running".into());
        }
        Ok((call, rx))
    }

    async fn finish_call(
        &self,
        plugin: &str,
        started: Result<(u64, oneshot::Receiver<Result<Value, String>>), String>,
        deadline: tokio::time::Instant,
        timeout: Duration,
    ) -> Result<Value, String> {
        let (call, rx) = started?;
        match tokio::time::timeout_at(deadline, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Plugin host is not running".into()),
            Err(_) => {
                self.inner.calls.lock().unwrap().remove(&call);
                Err(format!("Plugin '{plugin}' did not answer within {} s", timeout.as_secs_f32()))
            }
        }
    }

    /// Asks the host to stop its plugins and exit, and kills it if it has not exited after `grace`.
    pub fn stop(&self, grace: Duration) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        if let Some(tx) = self.inner.tx.lock().unwrap().take() {
            let _ = tx.send(ToHost::Shutdown);
        }
        let Some(mut child) = self.inner.child.lock().unwrap().take() else { return };
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        if self.inner.child.lock().unwrap().is_some() {
            let inner = self.inner.clone();
            std::thread::spawn(move || Supervisor { inner }.stop(Duration::from_millis(1500)));
        } else {
            self.inner.stopping.store(true, Ordering::SeqCst);
            self.inner.tx.lock().unwrap().take();
        }
    }
}

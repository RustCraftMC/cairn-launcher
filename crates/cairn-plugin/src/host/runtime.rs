//! One running plugin: a QuickJS runtime on its own thread. The thread waits for messages, timers
//! and finished async operations, and calls into JS for each one ("a turn"). Every turn has a CPU
//! budget; code that exceeds it is interrupted so the plugin keeps serving other events.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{CaughtError, Context, Ctx, Exception, Function, Module, Object, Persistent, Runtime};
use serde_json::{Map, Value, json};

use super::Shared;
use super::ops::{self, Storage};
use crate::manifest::{self, Permission};
use crate::protocol::{FromHost, LoadSpec, LogLevel, Target};

const SDK: &str = include_str!("sdk.js");
const MEMORY_LIMIT: usize = 128 * 1024 * 1024;
const STACK_LIMIT: usize = 1024 * 1024;
/// CPU time one turn may take before it is interrupted.
const TURN_BUDGET: Duration = Duration::from_secs(5);
const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(15);
/// Widget updates are sent at most this often; updates in between are merged.
const WIDGET_INTERVAL: Duration = Duration::from_millis(50);
const STORAGE_INTERVAL: Duration = Duration::from_secs(1);
const MIN_REPEAT: Duration = Duration::from_millis(10);
const LOGS_PER_SECOND: u32 = 50;
const MAX_LOG: usize = 8 * 1024;
const MAX_MODULE: u64 = 4 * 1024 * 1024;
const INTERRUPTED: &str = "Script interrupted";
/// Launcher pages a plugin may open; `plugin` is the plugin's own page in the manager.
const PAGES: [&str; 6] = ["settings", "extensions", "instances", "content", "accounts", "plugin"];
const MAX_CLIPBOARD: usize = 1024 * 1024;

pub(super) enum Msg {
    Event { name: String, data: Arc<str> },
    Invoke { call: u64, target: Target, data: String },
    Reply { request: u64, result: Result<Value, String> },
    OpDone { op: u32, result: Result<Value, String> },
    Stop,
}

/// A widget's merged data and latest view, sent at the next flush.
type WidgetUpdate = (Option<Map<String, Value>>, Option<Value>);

/// State reached from native functions. Never borrowed while JS runs from the event loop.
struct State {
    spec: LoadSpec,
    out: Sender<FromHost>,
    mailbox: Sender<Msg>,
    shared: Arc<Shared>,
    timers: BTreeSet<(Instant, u32)>,
    repeat: HashMap<u32, Option<Duration>>,
    next_timer: u32,
    next_op: u32,
    requests: HashMap<u64, u32>,
    /// Invocations whose result has not been sent yet.
    calls: BTreeSet<u64>,
    outbox: BTreeMap<String, WidgetUpdate>,
    widgets_sent: Option<Instant>,
    storage: Storage,
    storage_due: Option<Instant>,
    activation: Option<Result<(), String>>,
    logs: (Instant, u32),
}

impl State {
    fn send(&self, msg: FromHost) {
        let _ = self.out.send(msg);
    }

    fn log(&mut self, level: LogLevel, mut message: String) {
        let now = Instant::now();
        if now.duration_since(self.logs.0) >= Duration::from_secs(1) {
            if self.logs.1 > LOGS_PER_SECOND {
                let dropped = self.logs.1 - LOGS_PER_SECOND;
                self.send(FromHost::Log { plugin: self.spec.id.clone(), level: LogLevel::Warn, message: format!("{dropped} log lines dropped") });
            }
            self.logs = (now, 0);
        }
        self.logs.1 += 1;
        if self.logs.1 > LOGS_PER_SECOND {
            return;
        }
        if message.len() > MAX_LOG {
            let mut end = MAX_LOG;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
            message.push('…');
        }
        self.send(FromHost::Log { plugin: self.spec.id.clone(), level, message });
    }

    /// Merges the `data` and `view` of a widget or panel update into the outbox, sent at the next flush.
    fn queue_update(&mut self, key: String, value: &Value) -> Result<Value, String> {
        let slot = self.outbox.entry(key).or_default();
        match value.get("data") {
            Some(Value::Object(data)) => slot.0.get_or_insert_with(Map::new).extend(data.clone()),
            Some(Value::Null) | None => {}
            Some(_) => return Err("Widget data must be an object".into()),
        }
        if let Some(view) = value.get("view").filter(|v| !v.is_null()) {
            slot.1 = Some(view.clone());
        }
        Ok(Value::Null)
    }

    /// Synchronous SDK operations.
    fn call(&mut self, method: &str, value: Value) -> Result<Value, String> {
        match method {
            "log" => {
                let level = match value.get("level").and_then(Value::as_str) {
                    Some("debug") => LogLevel::Debug,
                    Some("warn") => LogLevel::Warn,
                    Some("error") => LogLevel::Error,
                    _ => LogLevel::Info,
                };
                self.log(level, value.get("message").and_then(Value::as_str).unwrap_or_default().to_string());
                Ok(Value::Null)
            }
            "widgets.update" => {
                let widget = value.get("widget").and_then(Value::as_str).unwrap_or_default();
                if !self.spec.widgets.iter().any(|w| w == widget) {
                    return Err(format!("Widget '{widget}' is not declared in plugin.json"));
                }
                self.queue_update(widget.to_string(), &value)
            }
            "panels.update" => {
                let panel = value.get("panel").and_then(Value::as_str).unwrap_or_default();
                if !self.spec.panels.iter().any(|p| p == panel) {
                    return Err(format!("Panel '{panel}' is not declared in plugin.json"));
                }
                // Panels share the outbox with widgets. The `panel:` prefix tells the launcher which one an update is for.
                self.queue_update(format!("panel:{panel}"), &value)
            }
            "panels.open" => {
                let panel = value.get("panel").and_then(Value::as_str).unwrap_or_default();
                if !self.spec.panels.iter().any(|p| p == panel) {
                    return Err(format!("Panel '{panel}' is not declared in plugin.json"));
                }
                self.send(FromHost::Notify { plugin: self.spec.id.clone(), method: "ui.open".into(), params: json!({ "page": format!("panel:{panel}") }) });
                Ok(Value::Null)
            }
            "events.subscribe" | "events.unsubscribe" => {
                let name = value.get("name").and_then(Value::as_str).unwrap_or_default();
                if name.is_empty() || name.len() > 64 {
                    return Err("Invalid event name".into());
                }
                self.send(FromHost::Notify { plugin: self.spec.id.clone(), method: method.into(), params: json!({ "name": name }) });
                Ok(Value::Null)
            }
            "ui.toast" | "ui.openUrl" | "ui.open" | "clipboard.write" => {
                let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default();
                match method {
                    "ui.openUrl" if !(text("url").starts_with("https://") || text("url").starts_with("http://")) => {
                        return Err("Only http and https links can be opened".into());
                    }
                    "ui.open" if !PAGES.contains(&text("page")) => return Err(format!("Unknown page '{}'; use one of {}", text("page"), PAGES.join(", "))),
                    "clipboard.write" if text("text").len() > MAX_CLIPBOARD => return Err("Clipboard text is limited to 1 MiB".into()),
                    _ => {}
                }
                self.send(FromHost::Notify { plugin: self.spec.id.clone(), method: method.into(), params: value });
                Ok(Value::Null)
            }
            m if m.starts_with("storage.") => {
                let result = self.storage.call(m, value);
                if self.storage.dirty && self.storage_due.is_none() {
                    self.storage_due = Some(Instant::now() + STORAGE_INTERVAL);
                }
                result
            }
            m if m.starts_with("fs.") => ops::fs_call(&self.spec, m, value),
            _ => Err(format!("Unknown operation '{method}'")),
        }
    }

    /// Asynchronous SDK operations. The result arrives later as [`Msg::OpDone`] or [`Msg::Reply`].
    fn start(&mut self, method: String, value: Value) -> u32 {
        self.next_op = self.next_op.wrapping_add(1);
        let op = self.next_op;
        let mailbox = self.mailbox.clone();
        let fail = |error: String| {
            let _ = mailbox.send(Msg::OpDone { op, result: Err(error) });
        };
        match method.as_str() {
            "http.request" | "http.download" => {
                let (client, spec) = (self.shared.http(), self.spec.clone());
                self.shared.runtime().spawn(async move {
                    let result = ops::http(client, spec, method, value).await;
                    let _ = mailbox.send(Msg::OpDone { op, result });
                });
            }
            "process.run" | "process.spawn" | "process.open" => {
                let spec = self.spec.clone();
                self.shared.runtime().spawn(async move {
                    let result = ops::process(spec, method, value).await;
                    let _ = mailbox.send(Msg::OpDone { op, result });
                });
            }
            "instances.list" | "instances.launch" | "launcher.info" => {
                if method.starts_with("instances.") && !self.spec.permissions.contains(&Permission::Instances) {
                    fail("Permission 'instances' was not granted".into());
                } else {
                    let request = self.shared.request(mailbox.clone());
                    self.requests.insert(request, op);
                    self.send(FromHost::Request { request, plugin: self.spec.id.clone(), method, params: value });
                }
            }
            _ => fail(format!("Unknown operation '{method}'")),
        }
        op
    }

    fn set_timer(&mut self, ms: f64, repeat: bool) -> u32 {
        self.next_timer = self.next_timer.wrapping_add(1);
        let id = self.next_timer;
        let delay = Duration::from_secs_f64((ms / 1000.0).clamp(0.0, 1e7));
        self.repeat.insert(id, repeat.then(|| delay.max(MIN_REPEAT)));
        self.timers.insert((Instant::now() + delay, id));
        id
    }

    fn clear_timer(&mut self, id: u32) {
        self.repeat.remove(&id);
        self.timers.retain(|&(_, t)| t != id);
    }

    fn flush_widgets(&mut self, force: bool) {
        if self.outbox.is_empty() || (!force && self.widgets_sent.is_some_and(|t| t.elapsed() < WIDGET_INTERVAL)) {
            return;
        }
        for (widget, (data, view)) in std::mem::take(&mut self.outbox) {
            self.send(FromHost::Widget { plugin: self.spec.id.clone(), widget, data, view });
        }
        self.widgets_sent = Some(Instant::now());
    }

    fn flush_storage(&mut self) {
        self.storage_due = None;
        if let Err(error) = self.storage.flush() {
            self.log(LogLevel::Error, format!("Saving storage failed: {error}"));
        }
    }

    fn next_wake(&self) -> Option<Instant> {
        let widgets = (!self.outbox.is_empty()).then(|| self.widgets_sent.map_or_else(Instant::now, |t| t + WIDGET_INTERVAL));
        [self.timers.first().map(|&(at, _)| at), widgets, self.storage_due].into_iter().flatten().min()
    }
}

/// Imports resolve relative to the importing module and never leave the package.
struct PackageResolver;

impl Resolver for PackageResolver {
    fn resolve<'js>(&mut self, _: &Ctx<'js>, base: &str, name: &str, _: Option<ImportAttributes<'js>>) -> rquickjs::Result<String> {
        let joined = if name.starts_with("./") || name.starts_with("../") {
            Path::new(base).parent().unwrap_or(Path::new("")).join(name)
        } else if base.is_empty() {
            PathBuf::from(name)
        } else {
            return Err(rquickjs::Error::new_resolving_message(base, name, "only relative imports inside the package are supported"));
        };
        let path = manifest::relative_path(&joined.to_string_lossy().replace('\\', "/"))
            .ok_or_else(|| rquickjs::Error::new_resolving_message(base, name, "path leaves the package"))?;
        Ok(path.to_string_lossy().replace('\\', "/"))
    }
}

struct PackageLoader {
    dir: PathBuf,
}

impl Loader for PackageLoader {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str, _: Option<ImportAttributes<'js>>) -> rquickjs::Result<Module<'js, Declared>> {
        let bytes = manifest::read_limited(&self.dir.join(name), MAX_MODULE).map_err(|e| rquickjs::Error::new_loading_message(name, e.to_string()))?;
        let mut source = String::from_utf8(bytes).map_err(|e| rquickjs::Error::new_loading_message(name, e.to_string()))?;
        if name.ends_with(".json") {
            source = format!("export default {source};");
        }
        Module::declare(ctx.clone(), name, source)
    }
}

fn describe<'js>(ctx: &Ctx<'js>, error: rquickjs::Error) -> String {
    match CaughtError::from_error(ctx, error) {
        CaughtError::Exception(e) => {
            let message = e.message().unwrap_or_default();
            if message == "interrupted" {
                return format!("{INTERRUPTED} after running longer than {} s", TURN_BUDGET.as_secs());
            }
            match e.stack().filter(|s| !s.trim().is_empty()) {
                Some(stack) => format!("{message}\n{}", stack.trim_end()),
                None => message,
            }
        }
        CaughtError::Value(v) => v.as_string().and_then(|s| s.to_string().ok()).unwrap_or_else(|| format!("{v:?}")),
        CaughtError::Error(e) => e.to_string(),
    }
}

/// Fields drop in order: JS handles before the context, the context before the runtime.
struct Plugin {
    host: Persistent<Object<'static>>,
    ctx: Context,
    rt: Runtime,
    state: Rc<RefCell<State>>,
    deadline: Rc<Cell<Option<Instant>>>,
    /// Set when the runtime ran out of memory; the plugin cannot be trusted to continue.
    fatal: Option<String>,
    /// Set when a turn was interrupted for exceeding its budget.
    interrupted: Option<String>,
}

pub(super) fn run(spec: LoadSpec, rx: Receiver<Msg>, mailbox: Sender<Msg>, out: Sender<FromHost>, shared: Arc<Shared>) {
    let id = spec.id.clone();
    let started = Instant::now();
    match Plugin::start(spec, mailbox, out.clone(), shared) {
        Ok(mut plugin) => plugin.serve(rx, started),
        Err(error) => {
            let _ = out.send(FromHost::Failed { plugin: id, error });
        }
    }
}

impl Plugin {
    fn start(spec: LoadSpec, mailbox: Sender<Msg>, out: Sender<FromHost>, shared: Arc<Shared>) -> Result<Self, String> {
        std::fs::create_dir_all(&spec.data_dir).map_err(|e| format!("{}: {e}", spec.data_dir.display()))?;
        let rt = Runtime::new().map_err(|e| e.to_string())?;
        rt.set_memory_limit(MEMORY_LIMIT);
        rt.set_max_stack_size(STACK_LIMIT);
        let deadline = Rc::new(Cell::new(None::<Instant>));
        let watch = deadline.clone();
        rt.set_interrupt_handler(Some(Box::new(move || watch.get().is_some_and(|at| Instant::now() >= at))));
        rt.set_loader(PackageResolver, PackageLoader { dir: spec.dir.clone() });
        let main = spec.main.replace('\\', "/");
        let info = json!({
            "id": spec.id,
            "name": spec.name,
            "version": spec.version,
            "dir": spec.dir.to_string_lossy(),
            "dataDir": spec.data_dir.to_string_lossy(),
            "permissions": spec.permissions.iter().map(|p| p.key()).collect::<Vec<_>>(),
            "commands": spec.commands,
            "widgets": spec.widgets,
            "panels": spec.panels,
            "settings": spec.settings,
        })
        .to_string();
        let state = Rc::new(RefCell::new(State {
            storage: Storage::new(&spec.data_dir),
            spec,
            out,
            mailbox,
            shared,
            timers: BTreeSet::new(),
            repeat: HashMap::new(),
            next_timer: 0,
            next_op: 0,
            requests: HashMap::new(),
            calls: BTreeSet::new(),
            outbox: BTreeMap::new(),
            widgets_sent: None,
            storage_due: None,
            activation: None,
            logs: (Instant::now(), 0),
        }));
        let tracker = state.clone();
        rt.set_host_promise_rejection_tracker(Some(Box::new(move |ctx, _promise, reason, handled| {
            if !handled {
                let text = describe(&ctx, ctx.throw(reason));
                if let Ok(mut state) = tracker.try_borrow_mut() {
                    state.log(LogLevel::Error, format!("Unhandled promise rejection: {text}"));
                }
            }
        })));
        let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
        let host = ctx.with(|ctx| {
            let build = || -> rquickjs::Result<Persistent<Object<'static>>> {
                let native = Object::new(ctx.clone())?;
                let st = state.clone();
                native.set(
                    "call",
                    Function::new(ctx.clone(), move |ctx: Ctx<'_>, method: String, json: String| -> rquickjs::Result<String> {
                        let value = serde_json::from_str(&json).map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
                        let result = st.borrow_mut().call(&method, value);
                        result.map(|v| v.to_string()).map_err(|e| Exception::throw_message(&ctx, &e))
                    })?,
                )?;
                let st = state.clone();
                native.set(
                    "start",
                    Function::new(ctx.clone(), move |ctx: Ctx<'_>, method: String, json: String| -> rquickjs::Result<u32> {
                        let value = serde_json::from_str(&json).map_err(|e| Exception::throw_message(&ctx, &e.to_string()))?;
                        Ok(st.borrow_mut().start(method, value))
                    })?,
                )?;
                let st = state.clone();
                native.set("timer", Function::new(ctx.clone(), move |ms: f64, repeat: bool| st.borrow_mut().set_timer(ms, repeat))?)?;
                let st = state.clone();
                native.set("clearTimer", Function::new(ctx.clone(), move |id: u32| st.borrow_mut().clear_timer(id))?)?;
                let st = state.clone();
                native.set(
                    "done",
                    Function::new(ctx.clone(), move |call: f64, ok: bool, json: String| {
                        let mut st = st.borrow_mut();
                        if st.calls.remove(&(call as u64)) {
                            let result = if ok { Ok(serde_json::from_str(&json).unwrap_or(Value::Null)) } else { Err(json) };
                            st.send(FromHost::Done { call: call as u64, result });
                        }
                    })?,
                )?;
                let st = state.clone();
                native.set(
                    "activated",
                    Function::new(ctx.clone(), move |ok: bool, error: String| {
                        st.borrow_mut().activation = Some(if ok { Ok(()) } else { Err(error) });
                    })?,
                )?;
                let sdk: Function = ctx.eval(SDK)?;
                let host: Object = sdk.call((native, ctx.json_parse(info)?))?;
                host.get::<_, Function>("activate")?.call::<_, ()>((main,))?;
                Ok(Persistent::save(&ctx, host))
            };
            build().map_err(|e| describe(&ctx, e))
        })?;
        Ok(Self { host, ctx, rt, state, deadline, fatal: None, interrupted: None })
    }

    /// Runs `f` against the SDK host object, then the microtasks it queued, within one CPU budget.
    fn turn(&mut self, f: impl for<'js> FnOnce(&Ctx<'js>, &Object<'js>) -> rquickjs::Result<()>) {
        self.deadline.set(Some(Instant::now() + TURN_BUDGET));
        let host = self.host.clone();
        let mut errors: Vec<String> = self.ctx.with(|ctx| match host.restore(&ctx).and_then(|host| f(&ctx, &host)) {
            Ok(()) => Vec::new(),
            Err(e) => vec![describe(&ctx, e)],
        });
        let mut graced = false;
        loop {
            // After an interruption, give the remaining jobs (usually the handlers that report the
            // error back) one short extra budget instead of interrupting them too.
            if !graced && self.deadline.get().is_some_and(|at| Instant::now() >= at) {
                graced = true;
                self.deadline.set(Some(Instant::now() + Duration::from_secs(1)));
            }
            match self.rt.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => break,
                Err(job) => errors.push(job.0.with(|ctx| describe(&ctx, rquickjs::Error::Exception))),
            }
        }
        self.deadline.set(None);
        let mut state = self.state.borrow_mut();
        for error in errors {
            if error.contains("out of memory") {
                self.fatal = Some(format!("Plugin exceeded its {} MiB memory limit", MEMORY_LIMIT >> 20));
            }
            if error.starts_with(INTERRUPTED) {
                // An interruption cannot be caught, so the promise behind an invocation never
                // settles. Fail the invocations that are still waiting instead.
                for call in std::mem::take(&mut state.calls) {
                    state.send(FromHost::Done { call, result: Err(error.clone()) });
                }
                self.interrupted = Some(error.clone());
            }
            state.log(LogLevel::Error, error);
        }
    }

    fn has_launch_hook(&mut self) -> bool {
        let host = self.host.clone();
        self.ctx.with(|ctx| host.restore(&ctx).and_then(|h| h.get::<_, Function>("hasLaunchHook")?.call(())).unwrap_or(false))
    }

    fn serve(&mut self, rx: Receiver<Msg>, started: Instant) {
        // The import and `activate()` were queued during start; run them as the first turn.
        self.turn(|_, _| Ok(()));
        let id = self.state.borrow().spec.id.clone();
        let mut active = false;
        let failed = loop {
            if let Some(error) = self.fatal.take() {
                break Some(error);
            }
            if !active {
                if let Some(error) = self.interrupted.take() {
                    break Some(error);
                }
                let activation = self.state.borrow_mut().activation.take();
                match activation {
                    Some(Ok(())) => {
                        active = true;
                        let launch_hook = self.has_launch_hook();
                        let millis = started.elapsed().as_secs_f64() * 1000.0;
                        self.state.borrow().send(FromHost::Activated { plugin: id.clone(), launch_hook, millis });
                    }
                    Some(Err(error)) => break Some(error),
                    None if started.elapsed() > ACTIVATE_TIMEOUT => {
                        break Some(format!("activate() did not finish within {} s", ACTIVATE_TIMEOUT.as_secs()));
                    }
                    None => {}
                }
            }
            let mut wake = self.state.borrow().next_wake();
            if !active {
                wake = wake.into_iter().chain([started + ACTIVATE_TIMEOUT]).min();
            }
            let msg = match wake {
                Some(at) => rx.recv_timeout(at.saturating_duration_since(Instant::now())),
                None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
            };
            match msg {
                Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => {
                    if active {
                        self.turn(|_, host| host.get::<_, Function>("deactivate")?.call(()));
                    }
                    break None;
                }
                Ok(Msg::Event { name, data }) => self.turn(|_, host| host.get::<_, Function>("dispatch")?.call((name, &*data))),
                Ok(Msg::Invoke { call, target, data }) => {
                    self.state.borrow_mut().calls.insert(call);
                    let (target, command) = match target {
                        Target::Command { id } => ("command", id),
                        Target::BeforeLaunch => ("before_launch", String::new()),
                    };
                    self.turn(|_, host| host.get::<_, Function>("invoke")?.call((call as f64, target, command, data)));
                }
                Ok(Msg::Reply { request, result }) => {
                    let op = self.state.borrow_mut().requests.remove(&request);
                    if let Some(op) = op {
                        self.complete(op, result);
                    }
                }
                Ok(Msg::OpDone { op, result }) => self.complete(op, result),
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.fire_timers();
            let mut state = self.state.borrow_mut();
            state.flush_widgets(false);
            if state.storage_due.is_some_and(|at| at <= Instant::now()) {
                state.flush_storage();
            }
        };
        let mut state = self.state.borrow_mut();
        state.flush_widgets(true);
        state.flush_storage();
        state.send(match failed {
            Some(error) => FromHost::Failed { plugin: id, error },
            None => FromHost::Stopped { plugin: id },
        });
    }

    fn complete(&mut self, op: u32, result: Result<Value, String>) {
        let (ok, json) = match result {
            Ok(value) => (true, value.to_string()),
            Err(error) => (false, error),
        };
        self.turn(|_, host| host.get::<_, Function>("complete")?.call((op, ok, json)));
    }

    fn fire_timers(&mut self) {
        let now = Instant::now();
        loop {
            let due = {
                let mut state = self.state.borrow_mut();
                match state.timers.first().copied() {
                    Some((at, id)) if at <= now => {
                        state.timers.pop_first();
                        let repeat = state.repeat.get(&id).copied().flatten();
                        match repeat {
                            // A late interval fires once and continues from now, instead of catching up.
                            Some(every) => {
                                state.timers.insert(((at + every).max(now + MIN_REPEAT), id));
                            }
                            None => {
                                state.repeat.remove(&id);
                            }
                        }
                        Some((id, repeat.is_some()))
                    }
                    _ => None,
                }
            };
            let Some((id, repeat)) = due else { break };
            self.turn(|_, host| host.get::<_, Function>("fire")?.call((id, repeat)));
        }
    }
}

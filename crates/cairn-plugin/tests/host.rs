//! Runs the real host loop over in-process pipes and drives it through the supervisor.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use cairn_plugin::{FromHost, Handler, LoadSpec, Permission, Supervisor, Target};
use serde_json::{Value, json};

struct Collect(mpsc::Sender<FromHost>);

impl Handler for Collect {
    fn message(&self, msg: FromHost) {
        let _ = self.0.send(msg);
    }
    fn exited(&self, _: String) {}
}

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("cairn-plugin-host-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Harness {
    sup: Supervisor,
    rx: mpsc::Receiver<FromHost>,
    rt: tokio::runtime::Runtime,
    dir: TestDir,
}

impl Harness {
    fn new() -> Self {
        let (host_in, sup_out) = std::io::pipe().unwrap();
        let (sup_in, host_out) = std::io::pipe().unwrap();
        std::thread::spawn(move || cairn_plugin::host::serve(host_in, host_out));
        let (tx, rx) = mpsc::channel();
        let sup = Supervisor::connect(sup_in, sup_out, Arc::new(Collect(tx)));
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let h = Self { sup, rx, rt, dir: TestDir::new() };
        assert!(matches!(h.next(), FromHost::Ready));
        h
    }

    fn next(&self) -> FromHost {
        self.rx.recv_timeout(Duration::from_secs(20)).expect("host message")
    }

    /// Waits for a message matching `pick`, skipping others (logs are printed to help debugging).
    fn wait<T>(&self, mut pick: impl FnMut(&FromHost) -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let msg = self.rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("expected host message");
            if let Some(found) = pick(&msg) {
                return found;
            }
            if let FromHost::Log { message, .. } = &msg {
                eprintln!("plugin log: {message}");
            }
        }
    }

    fn load(&self, id: &str, script: &str, permissions: Vec<Permission>) -> LoadSpec {
        self.load_with_panels(id, script, permissions, &[])
    }

    fn load_with_panels(&self, id: &str, script: &str, permissions: Vec<Permission>, panels: &[&str]) -> LoadSpec {
        let dir = self.dir.0.join("plugins").join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.js"), script).unwrap();
        let spec = LoadSpec {
            id: id.into(),
            name: id.into(),
            version: "1.0.0".into(),
            dir,
            main: "main.js".into(),
            data_dir: self.dir.0.join("plugin-data").join(id),
            permissions,
            commands: vec!["run".into(), "spin".into(), "fill".into(), "reject".into(), "open".into(), "info".into()],
            widgets: vec!["clock".into()],
            panels: panels.iter().map(|p| p.to_string()).collect(),
            settings: serde_json::Map::from_iter([("greeting".to_string(), json!("hello"))]),
        };
        assert!(self.sup.load(spec.clone()));
        spec
    }

    fn activated(&self, id: &str) -> bool {
        self.wait(|m| match m {
            FromHost::Activated { plugin, launch_hook, .. } if plugin == id => Some(*launch_hook),
            FromHost::Failed { plugin, error } if plugin == id => panic!("{id} failed: {error}"),
            _ => None,
        })
    }

    fn invoke(&self, id: &str, command: &str, data: Value) -> Result<Value, String> {
        self.rt.block_on(self.sup.invoke(id, Target::Command { id: command.into() }, data, Duration::from_secs(15)))
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn commands_modules_widgets_timers_and_storage() {
    let h = Harness::new();
    write(&h.dir.0.join("plugins/demo/lib/util.js"), "export const double = (x) => x * 2;\n");
    write(&h.dir.0.join("plugins/demo/data.json"), r#"{"greeting":"hi"}"#);
    let spec = h.load(
        "demo",
        r#"
        import { double } from "./lib/util.js";
        import data from "./data.json";
        let ticks = 0;
        export async function activate(cairn) {
            await new Promise((resolve) => setTimeout(resolve, 5));
            cairn.commands.register("run", async (args) => {
                cairn.storage.set("count", (cairn.storage.get("count", 0)) + 1);
                cairn.fs.writeText("notes/a.txt", data.greeting);
                return { doubled: double(args.n), text: cairn.fs.readText("notes/a.txt"), count: cairn.storage.get("count") };
            });
            const id = setInterval(() => {
                ticks += 1;
                cairn.widgets.setData("clock", { ticks: String(ticks) });
                if (ticks === 3) clearInterval(id);
            }, 10);
            cairn.events.on("ping", (e) => cairn.widgets.setData("clock", { ping: e.value }));
        }
        "#,
        vec![],
    );
    assert!(!h.activated("demo"));
    assert_eq!(h.invoke("demo", "run", json!({"n": 21})).unwrap(), json!({"doubled": 42, "text": "hi", "count": 1}));
    let ticks = h.wait(|m| match m {
        FromHost::Widget { data: Some(data), .. } if data.get("ticks") == Some(&json!("3")) => Some(3),
        _ => None,
    });
    assert_eq!(ticks, 3);
    assert!(h.sup.event(None, "ping", json!({"value": "pong"})));
    h.wait(|m| matches!(m, FromHost::Widget { data: Some(d), .. } if d.get("ping") == Some(&json!("pong"))).then_some(()));
    assert!(h.invoke("demo", "missing", json!({})).unwrap_err().contains("not registered"));
    // Storage is written when the plugin stops, so a restart sees the count.
    assert!(h.sup.unload("demo"));
    h.wait(|m| matches!(m, FromHost::Stopped { plugin } if plugin == "demo").then_some(()));
    assert_eq!(std::fs::read_to_string(spec.data_dir.join("storage.json")).unwrap(), r#"{"count":1}"#);
    assert!(h.sup.load(spec));
    h.activated("demo");
    assert_eq!(h.invoke("demo", "run", json!({"n": 1})).unwrap()["count"], json!(2));
}

#[test]
fn plugin_panels_are_filled_and_opened() {
    let h = Harness::new();
    h.load_with_panels(
        "panelist",
        r#"
        export function activate(cairn) {
            cairn.commands.register("fill", () => {
                cairn.panels.setView("news", [{ element: "text", text: "Hello" }]);
                cairn.panels.setData("news", { count: "2" });
            });
            cairn.commands.register("reject", () => {
                const messages = [];
                const attempts = [() => cairn.panels.setData("nope", {}), () => cairn.panels.setView("nope", []), () => cairn.panels.open("nope")];
                for (const attempt of attempts) {
                    try { attempt(); messages.push("allowed"); } catch (e) { messages.push(e.message); }
                }
                return messages;
            });
            cairn.commands.register("open", () => cairn.panels.open("news"));
            cairn.commands.register("info", () => cairn.plugin.panels);
        }
        "#,
        vec![],
        &["news"],
    );
    h.activated("panelist");
    assert_eq!(h.invoke("panelist", "info", json!(null)).unwrap(), json!(["news"]));
    let messages: Vec<String> = serde_json::from_value(h.invoke("panelist", "reject", json!(null)).unwrap()).unwrap();
    assert_eq!(messages, ["Panel 'nope' is not declared in plugin.json"; 3]);

    h.invoke("panelist", "fill", json!(null)).unwrap();
    let (data, view) = h.wait(|m| match m {
        FromHost::Widget { widget, data, view, .. } if widget == "panel:news" => Some((data.clone(), view.clone())),
        _ => None,
    });
    let data = data.expect("panel data");
    assert_eq!(data["count"], json!("2"));
    assert_eq!(view, Some(json!([{ "element": "text", "text": "Hello" }])));

    h.invoke("panelist", "open", json!(null)).unwrap();
    let page = h.wait(|m| match m {
        FromHost::Notify { method, params, .. } if method == "ui.open" => Some(params.clone()),
        _ => None,
    });
    assert_eq!(page, json!({"page": "panel:news"}));
}

#[test]
fn permissions_are_enforced() {
    let h = Harness::new();
    h.load(
        "sandboxed",
        r#"
        export function activate(cairn) {
            cairn.commands.register("run", async () => {
                const errors = [];
                const attempt = async (f) => { try { await f(); errors.push("allowed"); } catch (e) { errors.push(String(e.message)); } };
                await attempt(() => cairn.fs.readText("../../outside.txt"));
                await attempt(() => cairn.http.get("http://127.0.0.1:9/"));
                await attempt(() => cairn.process.run("cmd", ["/c", "echo"]));
                await attempt(() => cairn.instances.list());
                await attempt(() => cairn.launch.onBeforeLaunch(() => ({})));
                await attempt(() => cairn.widgets.setData("other", {}));
                await attempt(() => cairn.ui.openUrl("file:///C:/Windows"));
                return errors;
            });
        }
        "#,
        vec![],
    );
    h.activated("sandboxed");
    let errors = h.invoke("sandboxed", "run", json!(null)).unwrap();
    let errors: Vec<String> = serde_json::from_value(errors).unwrap();
    assert_eq!(errors.len(), 7);
    assert!(errors.iter().all(|e| e != "allowed"), "{errors:?}");
    assert!(
        errors[0].contains("'fs'") && errors[1].contains("'network'") && errors[2].contains("'process'") && errors[3].contains("'instances'"),
        "{errors:?}"
    );
}

#[test]
fn granted_capabilities_work() {
    let h = Harness::new();
    let outside = h.dir.0.join("outside.txt");
    std::fs::write(&outside, "outside").unwrap();
    h.load(
        "trusted",
        r#"
        export function activate(cairn) {
            cairn.launch.onBeforeLaunch((ctx) => ({ addJvmArgs: ["-Dplugin=" + ctx.instance.id], memoryMib: 4096 }));
            cairn.commands.register("run", async (args) => {
                const text = cairn.fs.readText(args.path);
                const shell = cairn.plugin.permissions.includes("process")
                    ? await cairn.process.run(args.shell[0], args.shell.slice(1))
                    : null;
                return { text, out: shell && shell.stdout.trim(), code: shell && shell.code };
            });
        }
        "#,
        vec![Permission::Fs, Permission::Process, Permission::Launch],
    );
    assert!(h.activated("trusted"));
    let shell = if cfg!(windows) { json!(["cmd", "/c", "echo plugin"]) } else { json!(["sh", "-c", "echo plugin"]) };
    let result = h.invoke("trusted", "run", json!({"path": outside.to_string_lossy(), "shell": shell})).unwrap();
    assert_eq!(result, json!({"text": "outside", "out": "plugin", "code": 0}));
    let patch = h.rt.block_on(h.sup.invoke("trusted", Target::BeforeLaunch, json!({"instance": {"id": "demo"}}), Duration::from_secs(5))).unwrap();
    assert_eq!(patch, json!({"addJvmArgs": ["-Dplugin=demo"], "memoryMib": 4096}));
}

#[test]
fn runaway_code_is_contained() {
    let h = Harness::new();
    h.load(
        "hostile",
        r#"
        export function activate(cairn) {
            cairn.commands.register("spin", () => { for (;;) {} });
            cairn.commands.register("run", () => "still alive");
        }
        "#,
        vec![],
    );
    h.activated("hostile");
    let started = Instant::now();
    let error = h.invoke("hostile", "spin", json!(null)).unwrap_err();
    assert!(error.contains("interrupted"), "{error}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(h.invoke("hostile", "run", json!(null)).unwrap(), json!("still alive"));

    h.load("hungry", "export function activate() { const keep = []; for (;;) keep.push(new Array(1e6).fill(1)); }", vec![]);
    let error = h.wait(|m| match m {
        FromHost::Failed { plugin, error } if plugin == "hungry" => Some(error.clone()),
        _ => None,
    });
    assert!(error.contains("memory"), "{error}");

    h.load("broken", "export function activate() { throw new Error('boom'); }", vec![]);
    let error = h.wait(|m| match m {
        FromHost::Failed { plugin, error } if plugin == "broken" => Some(error.clone()),
        _ => None,
    });
    assert!(error.contains("boom"), "{error}");
    h.load("escape", "import x from '../demo/main.js'; export function activate() {}", vec![]);
    h.wait(|m| matches!(m, FromHost::Failed { plugin, .. } if plugin == "escape").then_some(()));
    // The host keeps serving the plugin that behaved.
    assert_eq!(h.invoke("hostile", "run", json!(null)).unwrap(), json!("still alive"));
}

#[test]
fn launcher_requests_are_routed_back() {
    let h = Harness::new();
    h.load(
        "asker",
        r#"
        export function activate(cairn) {
            cairn.commands.register("run", async () => (await cairn.instances.list()).map((i) => i.id));
        }
        "#,
        vec![Permission::Instances],
    );
    h.activated("asker");
    let (sup, rt) = (&h.sup, &h.rt);
    let call = std::thread::scope(|s| {
        let pending = s.spawn(move || rt.block_on(sup.invoke("asker", Target::Command { id: "run".into() }, json!(null), Duration::from_secs(15))));
        let request = h.wait(|m| match m {
            FromHost::Request { request, method, .. } if method == "instances.list" => Some(*request),
            _ => None,
        });
        h.sup.reply(request, Ok(json!([{"id": "a"}, {"id": "b"}])));
        pending.join().unwrap()
    });
    assert_eq!(call.unwrap(), json!(["a", "b"]));
}

/// The examples in `docs/plugins/examples` must keep working.
#[test]
fn documented_examples_run() {
    let h = Harness::new();
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/plugins/examples");
    let load = |folder: &str| {
        let manifest = cairn_plugin::Manifest::parse(&std::fs::read(examples.join(folder).join("plugin.json")).unwrap()).unwrap();
        let spec = LoadSpec {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            dir: examples.join(folder),
            main: manifest.main.clone().unwrap(),
            data_dir: h.dir.0.join("plugin-data").join(&manifest.id),
            permissions: manifest.permissions.iter().copied().collect(),
            commands: manifest.commands.iter().map(|c| c.id.clone()).collect(),
            widgets: manifest.widgets.iter().map(|w| w["id"].as_str().unwrap().to_string()).collect(),
            panels: manifest.panels.iter().map(|p| p["id"].as_str().unwrap().to_string()).collect(),
            settings: cairn_plugin::manifest::load_settings(&h.dir.0, &manifest),
        };
        assert!(h.sup.load(spec));
        manifest.id
    };
    let clock = load("hello-clock");
    assert!(!h.activated(&clock));
    let typed = json!({"widget": "clock", "key": "greeting", "value": "Hi"});
    assert_eq!(h.invoke(&clock, "set-greeting", typed).unwrap(), json!("Greeting saved"));
    h.wait(|m| matches!(m, FromHost::Widget { data: Some(d), .. } if d.get("greeting") == Some(&json!("Hi ×1"))).then_some(()));
    assert_eq!(h.invoke(&clock, "reset", json!({"widget": "clock", "args": null})).unwrap(), json!("Counter reset"));

    let tuner = load("launch-tuner");
    assert!(h.activated(&tuner), "launch-tuner registers a launch handler");
    let ctx = json!({
        "instance": {"id": "1.20.1", "name": "1.20.1", "mcVersion": "1.20.1", "loaders": [], "gameDir": "C:/game"},
        "java": {"path": "C:/java/bin/java.exe", "version": "17.0.9", "major": 17},
        "memoryMib": 4096, "jvmArgs": [], "gameArgs": [], "window": {"width": 854, "height": 480, "fullscreen": false}, "server": null, "dryRun": true
    });
    let patch = h.rt.block_on(h.sup.invoke(&tuner, Target::BeforeLaunch, ctx, Duration::from_secs(5))).unwrap();
    assert_eq!(patch, json!({"addJvmArgs": ["-Dfile.encoding=UTF-8", "-Dstdout.encoding=UTF-8"], "window": {"width": 1280, "height": 720}}));
    assert!(h.sup.event(Some(&tuner), "game.started", json!({"game": 1, "instance": "1.20.1", "pid": 42})));
    assert!(h.sup.event(Some(&tuner), "game.exited", json!({"game": 1, "instance": "1.20.1", "code": 0, "crashed": false})));
    h.wait(|m| {
        matches!(m, FromHost::Widget { data: Some(d), .. } if d.get("result").and_then(Value::as_str).is_some_and(|r| r.starts_with("Exited"))).then_some(())
    });

    // server-status needs the network; only check that it loads and activates.
    let status = load("server-status");
    h.activated(&status);
}

#[test]
fn settings_and_event_subscriptions() {
    let h = Harness::new();
    h.load(
        "configured",
        r#"
        export function activate(cairn) {
            const seen = [];
            cairn.settings.onChange((e) => seen.push(e.key));
            const off = cairn.events.on("game.output", () => {});
            cairn.commands.register("run", () => ({ greeting: cairn.settings.get("greeting"), missing: cairn.settings.get("nope", 7), seen, all: cairn.settings.all() }));
            cairn.commands.register("spin", () => off());
        }
        "#,
        vec![],
    );
    h.wait(|m| match m {
        FromHost::Notify { method, params, .. } if method == "events.subscribe" => {
            assert_eq!(params["name"], "game.output");
            Some(())
        }
        _ => None,
    });
    h.activated("configured");
    assert_eq!(h.invoke("configured", "run", json!(null)).unwrap(), json!({"greeting": "hello", "missing": 7, "seen": [], "all": {"greeting": "hello"}}));
    assert!(h.sup.event(Some("configured"), "settings.changed", json!({"key": "greeting", "values": {"greeting": "hi"}})));
    assert_eq!(h.invoke("configured", "run", json!(null)).unwrap(), json!({"greeting": "hi", "missing": 7, "seen": ["greeting"], "all": {"greeting": "hi"}}));
    h.invoke("configured", "spin", json!(null)).unwrap();
    h.wait(|m| matches!(m, FromHost::Notify { method, .. } if method == "events.unsubscribe").then_some(()));
}

#[test]
fn a_failed_import_is_reported_once() {
    let h = Harness::new();
    h.load("broken-import", "import x from './missing.js'; export function activate() {}", vec![]);
    let mut logs = Vec::new();
    let error = h.wait(|m| match m {
        FromHost::Failed { plugin, error } if plugin == "broken-import" => Some(error.clone()),
        FromHost::Log { message, .. } => {
            logs.push(message.clone());
            None
        }
        _ => None,
    });
    assert!(error.contains("missing.js"), "{error}");
    assert!(logs.iter().all(|l| !l.contains("Unhandled")), "{logs:?}");
}

#[test]
fn template_runs_and_types_cover_the_sdk() {
    let h = Harness::new();
    cairn_plugin::template::create(&h.dir.0, "my-plugin", "My plugin").unwrap();
    let entry = cairn_plugin::manifest::scan(&h.dir.0).unwrap().remove(0);
    assert!(h.sup.load(entry.load_spec(&h.dir.0).unwrap()));
    h.activated("my-plugin");
    assert_eq!(h.invoke("my-plugin", "say-hello", json!({})).unwrap(), json!("Hello from My plugin!"));

    // Every member of the live `cairn` object must be declared in cairn.d.ts.
    h.load(
        "lister",
        r#"
        export function activate(cairn) {
            cairn.commands.register("run", () => {
                const names = [];
                for (const [key, value] of Object.entries(cairn)) {
                    names.push(key);
                    if (value && typeof value === "object") for (const member of Object.keys(value)) names.push(`${key}.${member}`);
                }
                return names;
            });
        }
        "#,
        vec![],
    );
    h.activated("lister");
    let names: Vec<String> = serde_json::from_value(h.invoke("lister", "run", json!(null)).unwrap()).unwrap();
    let types = cairn_plugin::template::TYPES;
    let declared = |name: &str| types.contains(&format!("{name}(")) || types.contains(&format!("{name}<")) || types.contains(&format!("{name}:"));
    assert!(names.len() > 50, "{names:?}");
    for name in names {
        let (outer, member) = name.split_once('.').map_or((name.as_str(), None), |(o, m)| (o, Some(m)));
        assert!(declared(outer), "cairn.{outer} is missing from cairn.d.ts");
        if let Some(member) = member {
            assert!(declared(member), "cairn.{name} is missing from cairn.d.ts");
        }
    }
}

#[test]
fn clipboard_pages_and_launch_options_reach_the_launcher() {
    let h = Harness::new();
    h.load(
        "helper",
        r#"
        export function activate(cairn) {
            cairn.commands.register("run", () => {
                let rejected = "";
                try { cairn.ui.open("home"); } catch (e) { rejected = e.message; }
                cairn.ui.open("plugin");
                cairn.clipboard.writeText("copied");
                return rejected;
            });
            cairn.commands.register("spin", () => cairn.instances.launch("1.20.1", { server: "mc.example.com", account: "Steve" }));
        }
        "#,
        vec![Permission::Instances],
    );
    h.activated("helper");
    assert!(h.invoke("helper", "run", json!(null)).unwrap().as_str().unwrap().contains("Unknown page 'home'"));
    let mut notified = Vec::new();
    while notified.len() < 2 {
        if let FromHost::Notify { method, params, .. } = h.next() {
            notified.push((method, params));
        }
    }
    assert_eq!(notified, [("ui.open".to_string(), json!({"page": "plugin"})), ("clipboard.write".to_string(), json!({"text": "copied"}))]);
    let (sup, rt) = (&h.sup, &h.rt);
    std::thread::scope(|s| {
        let pending = s.spawn(move || rt.block_on(sup.invoke("helper", Target::Command { id: "spin".into() }, json!(null), Duration::from_secs(15))));
        let (request, params) = h.wait(|m| match m {
            FromHost::Request { request, method, params, .. } if method == "instances.launch" => Some((*request, params.clone())),
            _ => None,
        });
        assert_eq!(params, json!({"id": "1.20.1", "server": "mc.example.com", "account": "Steve"}));
        h.sup.reply(request, Ok(Value::Null));
        pending.join().unwrap().unwrap();
    });
}

// Plugin SDK. The host evaluates this before any plugin module and calls the returned function with
// its native bindings and the plugin's details. The returned object is the host's way back into JS.
(function (native, info) {
  "use strict";

  const listeners = new Map();
  const commands = new Map();
  const pending = new Map();
  const timers = new Map();
  let beforeLaunch = null;
  let main = null;
  let settings = { ...info.settings };

  const toJson = (value) => JSON.stringify(value === undefined ? null : value);
  const call = (method, args) => {
    const out = native.call(method, toJson(args));
    return out === undefined ? undefined : JSON.parse(out);
  };
  const start = (method, args) =>
    new Promise((resolve, reject) => {
      pending.set(native.start(method, toJson(args)), { resolve, reject });
    });

  const describe = (value) => {
    if (typeof value === "string") return value;
    if (value instanceof Error) return value.stack ? `${value}\n${value.stack}` : String(value);
    try {
      const json = JSON.stringify(value);
      return json === undefined ? String(value) : json;
    } catch (_) {
      return String(value);
    }
  };
  const log = (level) => (...args) => call("log", { level, message: args.map(describe).join(" ") });
  const report = (where) => (error) => log("error")(`${where}:`, error);
  const requirePermission = (permission) => {
    if (!info.permissions.includes(permission)) throw new Error(`Permission '${permission}' is not declared in plugin.json`);
  };
  const run = (where, fn, arg) => {
    try {
      const result = fn(arg);
      if (result && typeof result.then === "function") result.then(undefined, report(where));
    } catch (error) {
      report(where)(error);
    }
  };

  class Response {
    constructor(raw) {
      this.status = raw.status;
      this.ok = raw.status >= 200 && raw.status < 300;
      this.url = raw.url;
      this.headers = raw.headers;
      this.body = raw.body;
    }
    text() {
      return Promise.resolve(this.body);
    }
    json() {
      return Promise.resolve().then(() => JSON.parse(this.body));
    }
  }

  const request = (options) => start("http.request", options).then((raw) => new Response(raw));
  const checkedJson = (response) => {
    if (!response.ok) throw new Error(`HTTP ${response.status} from ${response.url}`);
    return response.json();
  };

  const cairn = {
    plugin: Object.freeze({ ...info, permissions: Object.freeze([...info.permissions]) }),
    log: log("info"),

    events: Object.freeze({
      on(name, fn) {
        if (typeof fn !== "function") throw new TypeError("events.on needs a function");
        if (!listeners.has(name)) listeners.set(name, new Set());
        const set = listeners.get(name);
        // The launcher only sends events someone listens to.
        if (set.size === 0 && name !== "settings.changed") call("events.subscribe", { name });
        set.add(fn);
        return () => {
          if (set.delete(fn) && set.size === 0 && name !== "settings.changed") call("events.unsubscribe", { name });
        };
      },
    }),

    settings: Object.freeze({
      get: (key, fallback) => (key in settings ? settings[key] : fallback),
      all: () => ({ ...settings }),
      onChange: (fn) => cairn.events.on("settings.changed", fn),
    }),

    commands: Object.freeze({
      register(id, fn) {
        if (!info.commands.includes(id)) throw new Error(`Command '${id}' is not declared in plugin.json`);
        if (typeof fn !== "function") throw new TypeError("commands.register needs a function");
        commands.set(id, fn);
        return () => commands.delete(id);
      },
    }),

    launch: Object.freeze({
      onBeforeLaunch(fn) {
        requirePermission("launch");
        if (typeof fn !== "function") throw new TypeError("launch.onBeforeLaunch needs a function");
        beforeLaunch = fn;
        return () => {
          if (beforeLaunch === fn) beforeLaunch = null;
        };
      },
    }),

    widgets: Object.freeze({
      setData(id, data) {
        call("widgets.update", { widget: id, data });
      },
      setView(id, view) {
        call("widgets.update", { widget: id, view });
      },
    }),

    panels: Object.freeze({
      setData(id, data) {
        call("panels.update", { panel: id, data });
      },
      setView(id, view) {
        call("panels.update", { panel: id, view });
      },
      open(id) {
        call("panels.open", { panel: id });
      },
    }),

    storage: Object.freeze({
      get: (key, fallback) => {
        const value = call("storage.get", { key });
        return value === null ? fallback : value;
      },
      set: (key, value) => call("storage.set", { key, value }),
      remove: (key) => call("storage.remove", { key }),
      keys: () => call("storage.keys"),
      clear: () => call("storage.clear"),
    }),

    fs: Object.freeze({
      resolve: (path) => call("fs.resolve", { path }),
      exists: (path) => call("fs.exists", { path }),
      stat: (path) => call("fs.stat", { path }),
      list: (path) => call("fs.list", { path }),
      readText: (path) => call("fs.readText", { path }),
      readJson: (path) => JSON.parse(call("fs.readText", { path })),
      readBase64: (path) => call("fs.readBase64", { path }),
      writeText: (path, text) => call("fs.writeText", { path, text: String(text) }),
      writeJson: (path, value) => call("fs.writeText", { path, text: JSON.stringify(value, null, 2) }),
      writeBase64: (path, data) => call("fs.writeBase64", { path, data }),
      appendText: (path, text) => call("fs.appendText", { path, text: String(text) }),
      mkdir: (path) => call("fs.mkdir", { path }),
      remove: (path, options) => call("fs.remove", { path, recursive: !!options?.recursive }),
      rename: (from, to) => call("fs.rename", { from, to }),
      copy: (from, to) => call("fs.copy", { from, to }),
    }),

    http: Object.freeze({
      request,
      get: (url, options) => request({ ...options, url, method: "GET" }),
      getJson: (url, options) => request({ ...options, url, method: "GET" }).then(checkedJson),
      post: (url, body, options) => request({ ...options, url, method: "POST", body: String(body) }),
      postJson: (url, value, options) =>
        request({
          ...options,
          url,
          method: "POST",
          body: JSON.stringify(value),
          headers: { "content-type": "application/json", ...options?.headers },
        }).then(checkedJson),
      download: (url, path, options) => start("http.download", { ...options, url, path }),
    }),

    process: Object.freeze({
      run: (program, args, options) => start("process.run", { ...options, program, args: args ?? [] }),
      spawn: (program, args, options) => start("process.spawn", { ...options, program, args: args ?? [] }),
      open: (target) => start("process.open", { target }),
    }),

    instances: Object.freeze({
      list: () => start("instances.list"),
      launch: (id, options) => start("instances.launch", { id, server: options?.server, account: options?.account }),
    }),

    launcher: Object.freeze({
      info: () => start("launcher.info"),
    }),

    ui: Object.freeze({
      toast: (text, kind) => call("ui.toast", { text: String(text), kind: kind ?? "info" }),
      openUrl: (url) => call("ui.openUrl", { url: String(url) }),
      open: (page) => call("ui.open", { page: String(page) }),
    }),

    clipboard: Object.freeze({
      writeText: (text) => call("clipboard.write", { text: String(text) }),
    }),
  };

  const schedule = (repeat) => (fn, ms, ...args) => {
    if (typeof fn !== "function") throw new TypeError("Timer callback must be a function");
    const id = native.timer(Math.max(0, Number(ms) || 0), repeat);
    timers.set(id, { fn, args });
    return id;
  };
  const clear = (id) => {
    if (timers.delete(id)) native.clearTimer(id);
  };

  globalThis.cairn = Object.freeze(cairn);
  globalThis.console = Object.freeze({ log: log("info"), info: log("info"), debug: log("debug"), warn: log("warn"), error: log("error") });
  globalThis.setTimeout = schedule(false);
  globalThis.setInterval = schedule(true);
  globalThis.clearTimeout = clear;
  globalThis.clearInterval = clear;
  globalThis.queueMicrotask ??= (fn) => Promise.resolve().then(fn);
  globalThis.fetch = (url, init) =>
    request({ url: String(url), method: init?.method ?? "GET", headers: init?.headers, body: init?.body === undefined ? undefined : String(init.body) });

  return {
    activate(entry) {
      // Importing here, with the handlers attached at once, reports a failed import only once.
      import(`./${entry}`)
        .then((ns) => {
          main = ns;
          return typeof ns.activate === "function" ? ns.activate(cairn) : undefined;
        })
        .then(
          () => native.activated(true, ""),
          (error) => native.activated(false, describe(error)),
        );
    },
    deactivate() {
      if (main && typeof main.deactivate === "function") run("deactivate", main.deactivate);
    },
    dispatch(name, json) {
      const data = JSON.parse(json);
      if (name === "settings.changed") settings = { ...data.values };
      const set = listeners.get(name);
      if (!set || set.size === 0) return;
      for (const fn of [...set]) run(`event ${name}`, fn, data);
    },
    invoke(id, target, command, json) {
      const fn = target === "before_launch" ? beforeLaunch : commands.get(command);
      Promise.resolve()
        .then(() => {
          if (!fn) throw new Error(target === "before_launch" ? "No launch handler is registered" : `Command '${command}' is not registered`);
          return fn(JSON.parse(json));
        })
        .then(
          (value) => native.done(id, true, toJson(value)),
          (error) => native.done(id, false, describe(error)),
        );
    },
    complete(id, ok, json) {
      const op = pending.get(id);
      if (!op) return;
      pending.delete(id);
      if (ok) op.resolve(JSON.parse(json));
      else op.reject(new Error(json));
    },
    fire(id, repeat) {
      const timer = timers.get(id);
      if (!timer) return;
      if (!repeat) timers.delete(id);
      run("timer", () => timer.fn(...timer.args));
    },
    hasLaunchHook() {
      return beforeLaunch !== null;
    },
  };
});

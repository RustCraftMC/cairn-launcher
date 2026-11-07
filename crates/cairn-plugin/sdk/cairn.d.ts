// Type definitions for the CairnLauncher plugin API, version 1.
// Reference this file from your plugin (`/// <reference path="./cairn.d.ts" />`) or keep the
// generated jsconfig.json, and editors will complete and check calls to `cairn`.

/** A JSON value, as stored, sent and received by the API. */
type CairnJson = null | boolean | number | string | CairnJson[] | { [key: string]: CairnJson };

/** Permissions a plugin declares in plugin.json and the user grants when enabling it. */
type CairnPermission = "network" | "fs" | "process" | "launch" | "instances" | "game";

interface CairnPluginInfo {
  readonly id: string;
  readonly name: string;
  readonly version: string;
  /** Package folder. */
  readonly dir: string;
  /** The plugin's own folder for files and storage. Relative `cairn.fs` paths start here. */
  readonly dataDir: string;
  readonly permissions: readonly CairnPermission[];
  readonly commands: readonly string[];
  readonly widgets: readonly string[];
  readonly panels: readonly string[];
  /** Setting values when the plugin started; use `cairn.settings` for current values. */
  readonly settings: Readonly<Record<string, CairnJson>>;
}

interface CairnLoader {
  kind: "fabric" | "quilt" | "forge" | "neo_forge" | "opti_fine" | "lite_loader";
  version: string;
}

interface CairnEvents {
  /** Needs `game`. */
  "game.started": { game: number; instance: string; pid: number };
  /** Needs `game`. */
  "game.exited": { game: number; instance: string | null; code: number | null; crashed: boolean };
  /** Needs `game`. Lines arrive in batches of about 50 ms. */
  "game.output": { game: number; instance: string | null; lines: { text: string; stderr: boolean }[] };
  /** Needs `game`. Sent after `game.exited` once the crash log has been analyzed. */
  "game.crashed": { game: number; instance: string; code: number | null; runSecs: number; causes: string[] };
  /** Needs `instances`. */
  "instances.changed": { ids: string[] };
  "account.changed": { account: { name: string; kind: string } | null };
  /** The user changed a setting; `key` is null after a reset. */
  "settings.changed": { key: string | null; values: Record<string, CairnJson> };
}

interface CairnFileStat {
  dir: boolean;
  file: boolean;
  size: number;
  /** Unix time in milliseconds. */
  modified: number | null;
}

interface CairnHttpOptions {
  method?: string;
  headers?: Record<string, string>;
  body?: string;
  /** Default 30 000, between 100 and 600 000. */
  timeoutMs?: number;
}

interface CairnResponse {
  readonly status: number;
  readonly ok: boolean;
  readonly url: string;
  readonly headers: Readonly<Record<string, string>>;
  /** Response text, up to 32 MiB. Use `cairn.http.download` for larger files. */
  readonly body: string;
  text(): Promise<string>;
  json<T = any>(): Promise<T>;
}

interface CairnProcessOptions {
  /** Default: the plugin's data folder. */
  cwd?: string;
  env?: Record<string, string>;
}

interface CairnInstance {
  id: string;
  name: string;
  mcVersion: string | null;
  loaders: CairnLoader[];
  gameDir: string;
  selected: boolean;
  running: boolean;
}

interface CairnLaunchContext {
  instance: { id: string; name: string; mcVersion: string | null; loaders: CairnLoader[]; gameDir: string };
  java: { path: string; version: string; major: number };
  memoryMib: number;
  /** The user's extra JVM arguments. */
  jvmArgs: string[];
  gameArgs: string[];
  window: { width: number; height: number; fullscreen: boolean };
  server: string | null;
  dryRun: boolean;
}

/** Changes to a launch. A single value set by two plugins is a conflict and stops the launch. */
interface CairnLaunchPatch {
  addJvmArgs?: string[];
  /** Removes the user's extra JVM arguments that equal one of these. */
  removeJvmArgs?: string[];
  addGameArgs?: string[];
  removeGameArgs?: string[];
  javaPath?: string;
  /** 128 to 1 048 576. */
  memoryMib?: number;
  /** Sizes 64 to 16 384. */
  window?: { width?: number; height?: number; fullscreen?: boolean };
  /** Server to join; an empty string joins none. */
  server?: string;
  env?: Record<string, string>;
}

/**
 * Data a command receives. Widget buttons pass `widget` and `args`; toggles and inputs pass `widget`, `key` and `value`.
 * Commands run from a panel get `panel` instead of `widget`; its toggles and inputs also pass `key` and `value`.
 */
interface CairnCommandData {
  widget?: string;
  panel?: string;
  args?: any;
  key?: string;
  value?: any;
  [key: string]: any;
}

/** One element of a widget view, as in widget YAML. */
type CairnViewItem =
  | { element: "text"; text: string; size?: number; weight?: "bold" | "normal"; color?: "accent" | "secondary" | "primary"; truncate?: boolean }
  | { element: "markdown"; content: string }
  | { element: "button"; text: string; icon?: string; style?: "primary" | "secondary"; action?: CairnAction }
  | { element: "banner"; title?: string; subtitle?: string; image?: string; height?: number; dim?: number; button_text?: string; action?: CairnAction }
  | { element: "card"; title?: string; icon?: string; children?: CairnViewItem[] }
  | { element: "grid"; columns?: number; gap?: number; children?: CairnViewItem[] }
  | { element: "row" | "column"; gap?: number; children?: CairnViewItem[] }
  | { element: "divider" }
  | { element: "progress"; value?: number; max?: number; label?: string; bind?: string }
  | { element: "badge"; text: string; color?: string }
  | { element: "toggle"; label: string; key: string; command: string }
  | { element: "input"; key: string; command: string; placeholder?: string; button?: string }
  /** A picture from an http(s) URL (PNG, JPEG or WebP), loaded by the launcher. It fills the width and is cropped to `height` (default 180). */
  | { element: "image"; url: string; height?: number; radius?: number }
  | { element: "segmented"; selected?: number; options: { label: string; action?: CairnAction }[] };

type CairnAction =
  | { type: "command"; command: string; args?: CairnJson }
  | { type: "launch"; instance?: string; server?: string }
  | { type: "open_url"; url: string }
  | { type: "open_sheet"; sheet: string }
  | { type: "open_panel"; panel: string }
  | { type: "copy"; text: string };

/** Launcher pages for `cairn.ui.open`; `plugin` is this plugin's page in the manager. */
type CairnPage = "settings" | "extensions" | "instances" | "content" | "accounts" | "plugin";

interface Cairn {
  readonly plugin: CairnPluginInfo;
  /** Writes to the plugin log in the manager. */
  log(...values: unknown[]): void;

  readonly events: {
    /** Listens to an event and returns a function that stops listening. */
    on<K extends keyof CairnEvents>(name: K, fn: (data: CairnEvents[K]) => unknown): () => void;
  };

  readonly settings: {
    /** Values are already checked against the setting's type in plugin.json. */
    get<T = any>(key: string, fallback?: T): T;
    all(): Record<string, CairnJson>;
    /** Runs after the user changes a setting; `get` already returns the new values. */
    onChange(fn: (change: CairnEvents["settings.changed"]) => unknown): () => void;
  };

  readonly commands: {
    /** Registers a command declared in plugin.json. A returned string is shown as a notification. */
    register(id: string, fn: (data: CairnCommandData) => unknown): () => void;
  };

  readonly launch: {
    /** Needs `launch`. Called before every launch, including dry runs; one handler per plugin. */
    onBeforeLaunch(fn: (ctx: CairnLaunchContext) => CairnLaunchPatch | null | undefined | Promise<CairnLaunchPatch | null | undefined>): () => void;
  };

  readonly widgets: {
    /** Merges values into a widget's data; `{data.key}` shows them and `null` removes a key. */
    setData(id: string, values: Record<string, string | number | boolean | null>): void;
    /** Replaces a widget's view. */
    setView(id: string, items: CairnViewItem[]): void;
  };

  readonly panels: {
    /** Merges values into a panel's data; `{data.key}` shows them and `null` removes a key. */
    setData(id: string, values: Record<string, string | number | boolean | null>): void;
    /** Replaces a panel's view. */
    setView(id: string, items: CairnViewItem[]): void;
    /** Opens one of this plugin's panels. */
    open(id: string): void;
  };

  readonly storage: {
    get<T = any>(key: string, fallback?: T): T;
    set(key: string, value: CairnJson): void;
    remove(key: string): void;
    keys(): string[];
    clear(): void;
  };

  /** Synchronous files. Relative paths are in the plugin's data folder; other paths need `fs`. */
  readonly fs: {
    resolve(path: string): string;
    exists(path: string): boolean;
    stat(path: string): CairnFileStat;
    list(path: string): (CairnFileStat & { name: string })[];
    readText(path: string): string;
    readJson<T = any>(path: string): T;
    readBase64(path: string): string;
    writeText(path: string, text: string): void;
    writeJson(path: string, value: unknown): void;
    writeBase64(path: string, data: string): void;
    appendText(path: string, text: string): void;
    mkdir(path: string): void;
    remove(path: string, options?: { recursive?: boolean }): void;
    rename(from: string, to: string): void;
    copy(from: string, to: string): void;
  };

  /** Needs `network`. */
  readonly http: {
    request(options: CairnHttpOptions & { url: string }): Promise<CairnResponse>;
    get(url: string, options?: CairnHttpOptions): Promise<CairnResponse>;
    /** Rejects unless the status is 2xx. */
    getJson<T = any>(url: string, options?: CairnHttpOptions): Promise<T>;
    post(url: string, body: string, options?: CairnHttpOptions): Promise<CairnResponse>;
    postJson<T = any>(url: string, value: unknown, options?: CairnHttpOptions): Promise<T>;
    /** Streams a file to `path` (same rules as `cairn.fs`). */
    download(url: string, path: string, options?: CairnHttpOptions): Promise<{ status: number; path: string; size: number }>;
  };

  /** Needs `process`. */
  readonly process: {
    run(
      program: string,
      args?: string[],
      options?: CairnProcessOptions & { stdin?: string; timeoutMs?: number },
    ): Promise<{ code: number | null; stdout: string; stderr: string }>;
    spawn(program: string, args?: string[], options?: CairnProcessOptions): Promise<{ pid: number | null }>;
    /** Opens a file or link with the system's default program. */
    open(target: string): Promise<null>;
  };

  /** Needs `instances`. */
  readonly instances: {
    list(): Promise<CairnInstance[]>;
    /** Starts the game as the launch button does. `account` is a name or id. */
    launch(id: string, options?: { server?: string; account?: string }): Promise<null>;
  };

  readonly launcher: {
    info(): Promise<{ version: string; language: string; selectedInstance: string | null; account: { name: string; kind: string } | null; dataDir: string | null }>;
  };

  readonly ui: {
    toast(text: string, kind?: "info" | "success" | "error"): void;
    /** Opens an http or https link in the browser. */
    openUrl(url: string): void;
    open(page: CairnPage): void;
  };

  readonly clipboard: {
    writeText(text: string): void;
  };
}

declare const cairn: Cairn;

declare function setTimeout(fn: (...args: any[]) => void, ms?: number, ...args: any[]): number;
declare function setInterval(fn: (...args: any[]) => void, ms?: number, ...args: any[]): number;
declare function clearTimeout(id: number): void;
declare function clearInterval(id: number): void;
declare function queueMicrotask(fn: () => void): void;
/** A simplified `fetch` on top of `cairn.http.request`. Needs `network`. */
declare function fetch(url: string, init?: CairnHttpOptions): Promise<CairnResponse>;
declare const console: { log(...v: unknown[]): void; info(...v: unknown[]): void; debug(...v: unknown[]): void; warn(...v: unknown[]): void; error(...v: unknown[]): void };

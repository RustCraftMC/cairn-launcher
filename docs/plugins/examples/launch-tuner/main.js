/** @param {Cairn} cairn */
export function activate(cairn) {
  const last = cairn.storage.get("last", { instance: "No game yet", result: "" });
  cairn.widgets.setData("last-session", last);

  cairn.launch.onBeforeLaunch((ctx) => {
    cairn.log(`Launching ${ctx.instance.name} (${ctx.instance.mcVersion}) with Java ${ctx.java.version}`);
    const patch = {};
    if (cairn.settings.get("utf8")) patch.addJvmArgs = ["-Dfile.encoding=UTF-8", "-Dstdout.encoding=UTF-8"];
    const size = cairn.settings.get("size");
    if (size !== "keep" && !ctx.window.fullscreen) {
      const [width, height] = size.split("x").map(Number);
      patch.window = { width, height };
    }
    return patch;
  });

  const remember = (session) => {
    cairn.storage.set("last", session);
    cairn.widgets.setData("last-session", session);
  };
  const started = new Map();
  cairn.events.on("game.started", (e) => started.set(e.game, Date.now()));
  cairn.events.on("game.exited", (e) => {
    const minutes = Math.round((Date.now() - (started.get(e.game) ?? Date.now())) / 60000);
    remember({ instance: e.instance ?? "Unknown instance", result: `${e.crashed ? "Crashed" : "Exited"} after ${minutes} min (code ${e.code})` });
  });
  // Arrives after game.exited once the crash log has been analyzed.
  cairn.events.on("game.crashed", (e) => {
    const causes = e.causes.length ? e.causes.join(", ") : "unknown cause";
    remember({ instance: e.instance, result: `Crashed after ${Math.round(e.runSecs / 60)} min: ${causes}` });
  });
}

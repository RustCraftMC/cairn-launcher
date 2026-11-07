/** @param {Cairn} cairn */
export function activate(cairn) {
  const refresh = async () => {
    const host = cairn.settings.get("host").trim();
    cairn.widgets.setData("status", { host, state: "Checking…" });
    try {
      const status = await cairn.http.getJson(`https://api.mcsrvstat.us/3/${encodeURIComponent(host)}`, { timeoutMs: 10000 });
      const online = status.players?.online ?? 0;
      const max = status.players?.max ?? 0;
      const state = status.online ? `${online} / ${max} online` : "Offline";
      // The progress bar shows how full the server is, as a percentage.
      const players = max > 0 ? Math.round((online / max) * 100) : 0;
      cairn.widgets.setData("status", { state, players, updated: `Updated ${new Date().toLocaleTimeString()}` });
      return state;
    } catch (error) {
      cairn.widgets.setData("status", { state: "Unavailable", players: 0, updated: String(error.message) });
      throw error;
    }
  };

  let timer = null;
  const schedule = () => {
    clearInterval(timer);
    timer = setInterval(() => refresh().catch(() => {}), cairn.settings.get("interval") * 1000);
  };
  refresh().catch(() => {});
  schedule();
  cairn.settings.onChange(() => {
    schedule();
    refresh().catch(() => {});
  });
  cairn.commands.register("refresh", refresh);
}

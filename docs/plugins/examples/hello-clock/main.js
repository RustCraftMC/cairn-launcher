const pad = (n) => String(n).padStart(2, "0");

/** @param {Cairn} cairn */
export function activate(cairn) {
  const show = () => {
    const now = new Date();
    const seconds = cairn.storage.get("seconds", true);
    cairn.widgets.setData("clock", {
      time: `${pad(now.getHours())}:${pad(now.getMinutes())}` + (seconds ? `:${pad(now.getSeconds())}` : ""),
      second: now.getSeconds(),
      seconds,
      greeting: `${cairn.storage.get("greeting", "Hello")} ×${cairn.storage.get("count", 0)}`,
    });
  };
  show();
  setInterval(show, 1000);

  cairn.commands.register("set-seconds", ({ value }) => {
    cairn.storage.set("seconds", value);
    show();
  });
  cairn.commands.register("set-greeting", ({ value }) => {
    cairn.storage.set("greeting", value.trim() || "Hello");
    cairn.storage.set("count", cairn.storage.get("count", 0) + 1);
    show();
    return "Greeting saved";
  });
  cairn.commands.register("reset", () => {
    cairn.storage.set("count", 0);
    show();
    return "Counter reset";
  });
}

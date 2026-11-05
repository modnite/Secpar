import { h, invoke, listen, type State } from "./api";
import { titlebar } from "./titlebar";
import { home } from "./views/home";
import { devices } from "./views/devices";
import { settings } from "./views/settings";
import { about } from "./views/about";

type Tab = "Home" | "Devices" | "Settings" | "About";
const tabs: Tab[] = ["Home", "Devices", "Settings", "About"];
let current: Tab = "Home";

const main = h("main");
const nav = h("nav.tabs", { role: "tablist" }, ...tabs.map((t) =>
  h("button.tab", { role: "tab", onclick: () => show(t) }, t)));

/** Shown instead of a view while the background service can't be reached. */
const down = () => h("section.view.down", {},
  h("div.down-mark", { "aria-hidden": "true" }),
  h("h2", {}, "The Secpar service isn't running"),
  h("p.lead", {}, "Restart the Secpar service or reopen Secpar Host."));

async function show(t: Tab) {
  current = t;
  const i = tabs.indexOf(t);
  nav.querySelectorAll(".tab").forEach((b, j) => b.setAttribute("aria-selected", String(i === j)));
  let view: HTMLElement;
  try {
    view =
      t === "Settings" ? await settings()
      : t === "About" ? about()
      : await (async () => {
        const s = await invoke<State>("get_state");
        return t === "Home" ? home(s) : devices(s, () => show("Devices"));
      })();
  } catch {
    view = down();
  }
  if (current === t) {
    main.dataset.view = view.classList.contains("down") ? "down" : t.toLowerCase();
    main.replaceChildren(view);
  }
}

document.getElementById("app")!.append(titlebar(), nav, main);
show("Home");

// The server pushes these on connect/disconnect, lock changes, and when a code was used.
listen("devices", () => (current === "Home" || current === "Devices") && show(current));
listen("paired", () => show(current === "Devices" ? "Devices" : "Home"));
listen<{ up: boolean }>("server", () => show(current));

import { h, invoke, type State } from "../api";

const TTL = 120;

function status(s: State): [string, string] {
  const n = s.devices.filter((d) => d.connected).length;
  if (s.locked && n) return ["warn", "Locked, phone connected"];
  if (n === 1) return ["live", "Phone connected"];
  if (n > 1) return ["live", `${n} phones connected`];
  return ["ok", "Ready for your phone"];
}

export function home(state: State): HTMLElement {
  const [tone, text] = status(state);
  const qr = h("div.qr", { "aria-label": "Pairing code" });
  const enlarge = () => {
    const dialog = h("dialog.pairing-dialog");
    const close = h("button.ghost", { onclick: () => (dialog as HTMLDialogElement).close() }, "Close");
    dialog.append(h("div.dialog-heading", {}, h("strong", {}, "Pair a device"), close), qr.cloneNode(true));
    dialog.addEventListener("close", () => dialog.remove());
    document.body.append(dialog);
    (dialog as HTMLDialogElement).showModal();
  };
  const left = h("span.countdown");
  let link = "";
  const copy = h("button.ghost", {
    onclick: async () => {
      await navigator.clipboard.writeText(link);
      copy.textContent = "Copied";
      setTimeout(() => (copy.textContent = "Copy pairing link"), 2500);
    },
  }, "Copy pairing link");
  let timer = 0;
  let expires = 0;
  const tick = () => {
    const s = Math.max(0, Math.round((expires - Date.now()) / 1000));
    left.textContent = `New code in ${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
    if (s === 0) refresh();
  };
  const refresh = async () => {
    const code = await invoke<{ svg: string; link: string }>("new_qr");
    qr.innerHTML = code.svg;
    const expanded = document.querySelector(".pairing-dialog .qr");
    if (expanded) expanded.innerHTML = code.svg;
    link = code.link;
    expires = Date.now() + TTL * 1000;
    tick();
  };
  clearInterval(timer);
  timer = window.setInterval(() => (document.contains(qr) ? tick() : clearInterval(timer)), 1000);
  refresh();

  const connected = state.devices.filter((d) => d.connected);
  const send = (id: string, folder: boolean) => invoke("send_file", { device: id, folder });
  return h(
    "section.view.home",
    {},
    h("div.hero", {},
      h(`span.pill.${tone}`, {}, h("i.dot"), text),
      h("h1.pc", {}, state.name)),
    h("div.pairing", {},
      h("button.code.qr-zoom", { "aria-label": "Enlarge pairing code", onclick: enlarge }, qr),
      h("div.pairing-actions", {},
        h("h2", {}, "Pair a device"),
        h("p.lead", {}, "Scan with Secpar"),
        copy, left)),
    connected.length
      ? h("ul.list.compact", {}, ...connected.map((d) => h("li.row", {}, h("i.dot.live"), h("span.grow.label", {}, d.name),
          h("button.ghost", { onclick: () => send(d.id, false) }, "Send a file"),
          h("button.ghost", { onclick: () => send(d.id, true) }, "Send folder"))))
      : null,
  );
}

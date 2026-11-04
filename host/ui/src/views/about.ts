import { h, invoke, type Update } from "../api";

export function about(): HTMLElement {
  const result = h("p.note", { role: "status" });
  const check = h("button.primary", {}, "Check for updates") as HTMLButtonElement;
  check.onclick = async () => {
    check.disabled = true;
    result.textContent = "Checking…";
    try {
      const u = await invoke<Update>("check_update");
      if (u.newer) {
        result.replaceChildren(`Version ${u.latest.replace(/^v/, "")} is available. `,
          h("a", { href: "#", onclick: (e: Event) => { e.preventDefault(); invoke("open_release", { url: u.url }); } }, "What's new"));
      } else {
        result.textContent = "You have the latest version.";
      }
    } catch {
      result.textContent = "Couldn't reach GitHub. Check your internet connection and try again.";
    }
    check.disabled = false;
  };
  return h(
    "section.view.about",
    {},
    h("img.about-wordmark", { src: "/wordmark.svg", alt: "Secpar" }),
    h("h2", {}, "Windows host"),
    h("p.hint", {}, `Version ${__APP_VERSION__}`),
    h("p.lead", {}, "Windows companion for Secpar."),
    h("p.hint", {}, "Preview build · GPL-3.0"),
    check,
    result,
  );
}

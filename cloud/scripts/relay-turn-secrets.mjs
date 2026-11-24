// Gives the relay Worker (pcr-rendezvous) the TURN key the self-hosted rendezvous already has,
// so hosts get relay credentials too. Run by .github/workflows/selfhost-tunnel.yml on the build
// box. The key is read from a private file there, sent only to Cloudflare, removed, never printed.
import { readFileSync, unlinkSync } from "node:fs";

const token = process.env.CLOUDFLARE_API_TOKEN;
if (!token) throw new Error("CLOUDFLARE_API_TOKEN is not set");
const IN = "/opt/secpar-accounts/inbox/turn-key";
const WORKER = "pcr-rendezvous";

async function api(path, method, body, step) {
  const r = await fetch(`https://api.cloudflare.com/client/v4${path}`, {
    method,
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok || j.success === false) {
    const why = (j.errors ?? []).map((e) => `${e.code} ${e.message}`).join("; ") || r.status;
    throw new Error(`${step}: ${why}`);
  }
  return j.result ?? j;
}

try {
  const vars = Object.fromEntries(readFileSync(IN, "utf8").split("\n").map((l) => l.trim()).filter((l) => l.includes("="))
    .map((l) => [l.slice(0, l.indexOf("=")), l.slice(l.indexOf("=") + 1)]));
  if (!vars.TURN_KEY_ID || !vars.TURN_KEY_TOKEN) throw new Error("the key file has no TURN_KEY_ID and TURN_KEY_TOKEN");
  const account = (await api("/accounts", "GET", undefined, "read the account"))[0].id;
  for (const name of ["TURN_KEY_ID", "TURN_KEY_TOKEN"]) {
    await api(`/accounts/${account}/workers/scripts/${WORKER}/secrets`, "PUT", { name, text: vars[name], type: "secret_text" }, `set ${name} (needs Workers Scripts: Edit)`);
  }
  console.log(`${WORKER} has TURN_KEY_ID and TURN_KEY_TOKEN.`);
} finally {
  try { unlinkSync(IN); } catch {}
}

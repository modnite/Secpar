// Moves accounts.secpar.win and relay.secpar.win from their Workers to the self-hosted services on
// the build box, through a Cloudflare Tunnel. Run by .github/workflows/selfhost-tunnel.yml on that
// box. The tunnel's connector token is written to a private file there and never printed.
import { writeFileSync } from "node:fs";

const token = process.env.CLOUDFLARE_API_TOKEN;
if (!token) throw new Error("CLOUDFLARE_API_TOKEN is not set");
const ZONE = "secpar.win";
const NAME = "secpar-home";
const ROUTES = [
  { hostname: "accounts.secpar.win", service: "http://127.0.0.1:8787", worker: "secpar-accounts" },
  { hostname: "relay.secpar.win", service: "http://127.0.0.1:8788", worker: "secpar-rendezvous" },
];
const OUT = "/opt/secpar-accounts/inbox/tunnel-token";

async function api(path, method = "GET", body, step) {
  const r = await fetch(`https://api.cloudflare.com/client/v4${path}`, {
    method,
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok || j.success === false) {
    const why = (j.errors ?? []).map((e) => `${e.code} ${e.message}`).join("; ") || r.status;
    throw new Error(`${step ?? path}: ${why}`);
  }
  return j.result;
}

const accounts = await api("/accounts", "GET", undefined, "read the account");
const account = accounts[0].id;
const zones = await api(`/zones?name=${ZONE}`, "GET", undefined, "read the zone");
const zone = zones[0].id;

// 1. The tunnel, made once and reused.
let tunnel = (await api(`/accounts/${account}/cfd_tunnel?name=${NAME}&is_deleted=false`, "GET", undefined, "list tunnels (needs Cloudflare Tunnel: Read)"))[0];
if (!tunnel) tunnel = await api(`/accounts/${account}/cfd_tunnel`, "POST", { name: NAME, config_src: "cloudflare" }, "create the tunnel (needs Cloudflare Tunnel: Edit)");
await api(`/accounts/${account}/cfd_tunnel/${tunnel.id}/configurations`, "PUT", {
  config: { ingress: [...ROUTES.map(({ hostname, service }) => ({ hostname, service, originRequest: {} })), { service: "http_status:404" }] },
}, "route the tunnel");
const connector = await api(`/accounts/${account}/cfd_tunnel/${tunnel.id}/token`, "GET", undefined, "read the tunnel token");
writeFileSync(OUT, String(connector), { mode: 0o600 });
console.log(`Tunnel ${NAME} is ready; its connector token is in ${OUT}.`);

// 2. The names leave their Workers (a Worker custom domain owns its DNS record).
const domains = await api(`/accounts/${account}/workers/domains`, "GET", undefined, "list Worker domains (needs Workers Scripts: Edit)");
for (const route of ROUTES) {
  for (const d of domains.filter((d) => d.hostname === route.hostname)) {
    await api(`/accounts/${account}/workers/domains/${d.id}`, "DELETE", undefined, `release ${route.hostname} from its Worker`);
    console.log(`Released ${route.hostname} from ${d.service}.`);
  }
}

// 3. DNS: each name points at the tunnel.
for (const route of ROUTES) {
  const records = await api(`/zones/${zone}/dns_records?name=${route.hostname}`, "GET", undefined, "list DNS records (needs DNS: Edit)");
  const target = `${tunnel.id}.cfargotunnel.com`;
  const same = records.find((r) => r.type === "CNAME" && r.content === target);
  for (const r of records.filter((r) => r !== same)) await api(`/zones/${zone}/dns_records/${r.id}`, "DELETE", undefined, `remove the old ${route.hostname} record`);
  if (!same) await api(`/zones/${zone}/dns_records`, "POST", { type: "CNAME", name: route.hostname, content: target, proxied: true, comment: "Secpar self-hosted" }, `point ${route.hostname} at the tunnel`);
  console.log(`${route.hostname} now goes through the tunnel.`);
}

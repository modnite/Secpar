import { readFile, mkdir, writeFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const accountProject = resolve(root, "cloud/accounts");
const relayProject = resolve(root, "cloud/rendezvous");
const siteProject = resolve(root, "cloud/site");
const { parse } = createRequire(resolve(accountProject, "package.json"))(
  "jsonc-parser",
);
const token = process.env.CLOUDFLARE_API_TOKEN;
async function api(path, method = "GET", body) {
  if (!token)
    throw new Error(
      "Set the Secpar repository secret CLOUDFLARE_API_TOKEN before deploying.",
    );
  const res = await fetch("https://api.cloudflare.com/client/v4" + path, {
    method,
    signal: AbortSignal.timeout(30000),
    headers: {
      authorization: `Bearer ${token}`,
      "content-type": "application/json",
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const result = await res.json();
  if (!res.ok || !result.success)
    throw new Error(
      `Cloudflare ${method} ${path.split("?")[0]} failed (${res.status}; codes ${(result.errors ?? []).map((e) => e.code).join(",")}). Check the token's account/zone permissions.`,
    );
  return result.result;
}
async function account() {
  const zones = await api("/zones?name=secpar.win");
  const zone = zones.find((z) => z.name === "secpar.win");
  if (!zone?.account?.id)
    throw new Error(
      "The token cannot see the secpar.win zone. Include Zone Read for secpar.win.",
    );
  if (zone.status !== "active")
    throw new Error(
      "secpar.win is not active in Cloudflare yet; retry once its zone is active.",
    );
  return zone.account.id;
}
async function config(project, changes) {
  const errors = [];
  const base = parse(
    await readFile(resolve(project, "wrangler.jsonc"), "utf8"),
    errors,
    { allowTrailingComma: true },
  );
  if (errors.length) throw new Error("Invalid source Worker configuration.");
  const output = { ...base, ...changes, main: resolve(project, base.main) };
  if (output.assets)
    output.assets = { ...output.assets, directory: resolve(project, output.assets.directory) };
  if (output.d1_databases)
    output.d1_databases = output.d1_databases.map((db) => ({
      ...db,
      migrations_dir: resolve(project, db.migrations_dir ?? "migrations"),
    }));
  const folder = resolve(project, ".wrangler/secpar-deploy");
  await mkdir(folder, { recursive: true });
  const path = resolve(folder, "wrangler.json");
  await writeFile(path, JSON.stringify(output, null, 2) + "\n");
  return path;
}
async function prepare() {
  const id = await account();
  const service = process.env.SECPAR_SERVICE ?? "accounts";
  if (service === "site") {
    // Blog comments: a D1 database and a Turnstile widget, created once and reused.
    const named = (await api(`/accounts/${id}/d1/database?name=secpar-blog`)).filter((d) => d.name === "secpar-blog");
    if (named.length > 1) throw new Error("More than one database matches secpar-blog.");
    const db = named[0] ?? (await api(`/accounts/${id}/d1/database`, "POST", { name: "secpar-blog" }));
    if (!db.uuid) throw new Error("Cloudflare did not return a D1 database id.");
    const widget = await blogWidget(id);
    const site = await config(siteProject, {
      account_id: id,
      d1_databases: [{ binding: "DB", database_name: "secpar-blog", database_id: db.uuid, migrations_dir: "migrations" }],
      ...(widget ? { vars: { TURNSTILE_SITEKEY: widget.sitekey } } : {}),
    });
    if (process.env.GITHUB_OUTPUT)
      await writeFile(process.env.GITHUB_OUTPUT, `site=${site}\n`, { flag: "a" });
    console.log("Prepared the Secpar public website for secpar.win and blog.secpar.win.");
    return;
  }
  if (!["accounts", "relay", "both"].includes(service))
    throw new Error("Unknown Secpar service.");
  let accounts;
  if (service !== "relay") {
    const databases = await api(
      `/accounts/${id}/d1/database?name=secpar-accounts`,
    );
    const matches = databases.filter((d) => d.name === "secpar-accounts");
    if (matches.length > 1)
      throw new Error(
        "More than one database matches secpar-accounts; select it explicitly before deployment.",
      );
    let db = matches[0];
    if (!db)
      db = await api(`/accounts/${id}/d1/database`, "POST", {
        name: "secpar-accounts",
      });
    if (!db.uuid)
      throw new Error("Cloudflare did not return a D1 database id.");
    accounts = await config(accountProject, {
      account_id: id,
      d1_databases: [
        {
          binding: "DB",
          database_name: "secpar-accounts",
          database_id: db.uuid,
          migrations_dir: "migrations",
        },
      ],
    });
  }
  const relay = await config(relayProject, {
    account_id: id,
    name: "secpar-rendezvous",
    workers_dev: false,
    routes: [{ pattern: "relay.secpar.win", custom_domain: true }],
  });
  if (process.env.GITHUB_OUTPUT)
    await writeFile(
      process.env.GITHUB_OUTPUT,
      `${accounts ? `accounts=${accounts}\n` : ""}relay=${relay}\n`,
      { flag: "a" },
    );
  console.log(
    "Prepared Secpar account and relay configurations for secpar.win.",
  );
}
async function blogWidget(id) {
  // Without Turnstile access the site still deploys, with comments closed.
  try { return await findOrCreateWidget(id); } catch (error) {
    console.log(`::warning::Blog comments stay closed: ${error.message} The token needs Account > Turnstile > Edit.`);
    return null;
  }
}
async function findOrCreateWidget(id) {
  const widgets = await api(`/accounts/${id}/challenges/widgets?per_page=50`);
  const found = widgets.find((w) => w.name === "Secpar blog comments");
  // The blog lives at secpar.win/blog; Turnstile matches the hostname.
  if (found && found.domains?.includes("secpar.win")) return found;
  if (found) {
    console.log("Allowed secpar.win on the blog Turnstile widget.");
    return api(`/accounts/${id}/challenges/widgets/${found.sitekey}`, "PUT", {
      name: found.name, domains: ["secpar.win"], mode: found.mode, region: found.region ?? "world",
    });
  }
  const created = await api(`/accounts/${id}/challenges/widgets`, "POST", {
    name: "Secpar blog comments", domains: ["secpar.win"], mode: "managed", region: "world",
  });
  console.log("Created the Turnstile widget for blog comments.");
  return created;
}
async function siteSecrets() {
  const id = await account();
  const key = process.env.BLOG_ADMIN_KEY;
  if (!key || key.length < 32) throw new Error("Set the repository secret BLOG_ADMIN_KEY (at least 32 characters).");
  let found;
  try {
    found = (await api(`/accounts/${id}/challenges/widgets?per_page=50`)).find((w) => w.name === "Secpar blog comments");
  } catch { /* reported by prepare */ }
  if (!found) { console.log("::warning::No blog Turnstile widget yet; comments stay closed."); return; }
  const widget = await api(`/accounts/${id}/challenges/widgets/${found.sitekey}`);
  const path = `/accounts/${id}/workers/scripts/secpar-site/secrets`;
  // Secrets go straight to Cloudflare's secret store: never to disk, logs or arguments.
  await api(path, "PUT", { name: "TURNSTILE_SECRET", type: "secret_text", text: widget.secret });
  await api(path, "PUT", { name: "BLOG_ADMIN_KEY", type: "secret_text", text: key });
  console.log("Configured the blog comment secrets.");
  // The blog editor signs in through a GitHub OAuth app.
  const clientId = process.env.BLOG_GITHUB_CLIENT_ID, clientSecret = process.env.BLOG_GITHUB_CLIENT_SECRET;
  if (!clientId || !clientSecret) { console.log("::warning::Blog editor sign-in is off: set BLOG_GITHUB_CLIENT_ID and BLOG_GITHUB_CLIENT_SECRET."); return; }
  await api(path, "PUT", { name: "BLOG_GITHUB_CLIENT_ID", type: "secret_text", text: clientId });
  await api(path, "PUT", { name: "BLOG_GITHUB_CLIENT_SECRET", type: "secret_text", text: clientSecret });
  console.log("Configured the blog editor sign-in.");
}
async function signingKey() {
  const id = await account();
  const path = `/accounts/${id}/workers/scripts/secpar-accounts/secrets`;
  const secrets = await api(path);
  if (secrets.some((s) => s.name === "GRANT_PRIVATE_JWK")) {
    console.log("Preserved the existing Secpar signing key.");
    return;
  }
  const pair = await crypto.subtle.generateKey(
    { name: "ECDSA", namedCurve: "P-256" },
    true,
    ["sign", "verify"],
  );
  const jwk = await crypto.subtle.exportKey("jwk", pair.privateKey);
  // No private key is written to disk, job output, command arguments or source control.
  await api(path, "PUT", {
    name: "GRANT_PRIVATE_JWK",
    type: "secret_text",
    text: JSON.stringify(jwk),
  });
  console.log(
    "Created the Secpar admission signing key in Cloudflare's secret store.",
  );
}
async function emailCheck() {
  const key = process.env.SECPAR_RESEND_API_KEY;
  if (!key) {
    console.log("Email sender check skipped: no email credential configured.");
    return;
  }
  const config = parse(await readFile(resolve(accountProject, "wrangler.jsonc"), "utf8"));
  const sender = config.vars?.EMAIL_FROM;
  if (!sender) throw new Error("No account email sender is configured.");
  // Resend's documented simulated-delivery address: no message reaches a person.
  // Never send an account credential, session or verification link in this probe.
  const result = await fetch("https://api.resend.com/emails", {
    method: "POST", signal: AbortSignal.timeout(10000),
    headers: { authorization: `Bearer ${key}`, "content-type": "application/json",
      "idempotency-key": `secpar-email-check-${process.env.GITHUB_RUN_ID ?? crypto.randomUUID()}` },
    body: JSON.stringify({ from: sender, to: ["delivered@resend.dev"],
      subject: "Secpar email configuration check",
      text: "Secpar email configuration check. This is a simulated-delivery test with no account data." }),
  });
  if (!result.ok) throw new Error(`Email sender check failed (HTTP ${result.status}). Check the Resend key's sending-domain restriction and mail.secpar.win verification.`);
  console.log("Resend accepted the configured Secpar sender and simulated-delivery test. Actual inbox delivery is not verified by this check.");
}
async function emailKey() {
  const key = process.env.SECPAR_RESEND_API_KEY;
  if (!key) {
    console.log(
      "Email delivery is not configured; registration stays disabled.",
    );
    return;
  }
  const id = await account();
  await api(`/accounts/${id}/workers/scripts/secpar-accounts/secrets`, "PUT", {
    name: "RESEND_API_KEY",
    type: "secret_text",
    text: key,
  });
  console.log(
    "Configured the email API credential in Cloudflare's secret store.",
  );
}
try {
  if (process.argv[2] === "prepare") await prepare();
  else if (process.argv[2] === "signing-key") await signingKey();
  else if (process.argv[2] === "email-key") await emailKey();
  else if (process.argv[2] === "email-check") await emailCheck();
  else if (process.argv[2] === "site-secrets") await siteSecrets();
  else throw new Error("Expected prepare, signing-key, email-check or email-key.");
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}

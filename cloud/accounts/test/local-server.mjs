// The account service on 127.0.0.1 with a throwaway in-memory database, for
// checking the app against it. No Cloudflare account, live data or email is used.
//
//   node test/local-server.mjs <port> <file for the test accounts>
//
// Two confirmed accounts are created through the real sign-up routes. Their
// random passwords are written to the file, never printed.
import { build } from "esbuild";
import { createServer } from "node:http";
import { DatabaseSync } from "node:sqlite";
import { readFileSync, readdirSync, writeFileSync, mkdtempSync } from "node:fs";
import { randomBytes } from "node:crypto";
import { fileURLToPath, pathToFileURL } from "node:url";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generateKeyPair, exportJWK } from "jose";

const port = Number(process.argv[2]), accountsFile = process.argv[3];
if (!port || !accountsFile) throw new Error("usage: local-server.mjs <port> <accounts file>");
// The service only accepts an https origin for itself, so it keeps this name
// while the listener is plain HTTP on this computer.
const origin = "https://accounts.secpar.test", address = `http://127.0.0.1:${port}`;

const bundle = join(mkdtempSync(join(tmpdir(), "secpar-accounts-")), "index.mjs");
await build({ entryPoints: [fileURLToPath(new URL("../src/index.ts", import.meta.url))], bundle: true, format: "esm", platform: "node", outfile: bundle, target: "es2022", logLevel: "error" });
const { handle } = await import(pathToFileURL(bundle).href);

const sqlite = new DatabaseSync(":memory:");
const migrations = new URL("../migrations/", import.meta.url);
for (const name of readdirSync(migrations).sort()) sqlite.exec(readFileSync(new URL(name, migrations), "utf8"));
const statement = (sql, values = []) => ({
  bind: (...next) => statement(sql, next),
  first: async () => sqlite.prepare(sql).get(...values) ?? null,
  all: async () => ({ results: sqlite.prepare(sql).all(...values) }),
  run: async () => ({ success: true, meta: { changes: Number(sqlite.prepare(sql).run(...values).changes) } }),
});
const DB = {
  prepare: (sql) => statement(sql),
  async batch(statements) {
    sqlite.exec("BEGIN");
    try {
      const out = [];
      for (const s of statements) out.push(await s.run());
      sqlite.exec("COMMIT");
      return out;
    } catch (e) {
      sqlite.exec("ROLLBACK");
      throw e;
    }
  },
};
const env = {
  DB,
  PUBLIC_ORIGIN: origin,
  GRANT_PRIVATE_JWK: JSON.stringify(await exportJWK((await generateKeyPair("ES256", { extractable: true })).privateKey)),
  RESEND_API_KEY: "local",
  EMAIL_FROM: "Secpar <local@secpar.test>",
};
// Stands in for the mail provider: keeps the code instead of sending an email.
let mailed = "";
const mail = async (_url, init) => {
  mailed = JSON.parse(init.body).subject.slice(0, 6);
  return new Response("{}", { status: 200 });
};
const post = async (path, body) => {
  const r = await handle(new Request(origin + path, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) }), env, mail);
  if (r.status >= 300) throw new Error(`${path} answered ${r.status} ${(await r.json()).error}`);
};
const accounts = [];
for (const handleName of ["alice", "bob"]) {
  const account = { handle: handleName, email: `${handleName}@secpar.test`, password: randomBytes(12).toString("hex") };
  await post("/v1/app/register", account);
  await post("/v1/app/verify", { email: account.email, code: mailed });
  accounts.push(account);
}
writeFileSync(accountsFile, JSON.stringify({ origin, address, accounts }));

createServer(async (incoming, outgoing) => {
  const chunks = [];
  for await (const chunk of incoming) chunks.push(chunk);
  const headers = new Headers();
  for (const [name, value] of Object.entries(incoming.headers)) if (typeof value === "string") headers.set(name, value);
  const hasBody = incoming.method !== "GET" && incoming.method !== "HEAD";
  const answer = await handle(new Request(origin + incoming.url, { method: incoming.method, headers, body: hasBody ? Buffer.concat(chunks) : undefined }), env, mail);
  outgoing.writeHead(answer.status, Object.fromEntries(answer.headers));
  outgoing.end(Buffer.from(await answer.arrayBuffer()));
}).listen(port, "127.0.0.1", () => console.log(`Account service for tests on ${address}`));

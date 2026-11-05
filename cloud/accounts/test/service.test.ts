import { beforeEach, describe, expect, it } from "vitest";
import { DatabaseSync } from "node:sqlite";
import { readFileSync } from "node:fs";
import { generateKeyPair, exportJWK, importJWK, jwtVerify } from "jose";
import { handle, hash, type Env } from "../src/index";

class Statement {
  constructor(
    readonly db: DatabaseSync,
    readonly sql: string,
    readonly values: (string | number | null)[] = [],
  ) {}
  bind(...values: (string | number | null)[]) {
    return new Statement(this.db, this.sql, values);
  }
  async first() {
    return this.db.prepare(this.sql).get(...this.values) ?? null;
  }
  async all() {
    return { results: this.db.prepare(this.sql).all(...this.values) };
  }
  async run() {
    const result = this.db.prepare(this.sql).run(...this.values);
    return { success: true, meta: { changes: Number(result.changes) } };
  }
}
class Database {
  db = new DatabaseSync(":memory:");
  constructor() {
    this.db.exec(
      readFileSync(
        new URL("../migrations/0001_accounts.sql", import.meta.url),
        "utf8",
      ),
    );
  }
  prepare(sql: string) {
    return new Statement(this.db, sql);
  }
  async batch(statements: Statement[]) {
    this.db.exec("BEGIN");
    try {
      const out = [];
      for (const s of statements) out.push(await s.run());
      this.db.exec("COMMIT");
      return out;
    } catch (e) {
      this.db.exec("ROLLBACK");
      throw e;
    }
  }
}
const base = "https://accounts.example.test";
let db: Database, env: Env;
const tokens = {
  alice: "a".repeat(64),
  bob: "b".repeat(64),
  mallory: "c".repeat(64),
};
async function call(
  path: string,
  actor: keyof typeof tokens | null = "alice",
  method = "GET",
  data?: unknown,
  extra: Record<string, string> = {},
) {
  const headers: Record<string, string> = { ...extra };
  if (actor) headers.authorization = `Bearer ${tokens[actor]}`;
  if (data !== undefined) headers["content-type"] = "application/json";
  return handle(
    new Request(base + path, {
      method,
      headers,
      body: data === undefined ? undefined : JSON.stringify(data),
    }),
    env,
  );
}
async function friendship() {
  expect((await call("/v1/friends/bob", "alice", "PUT")).status).toBe(202);
  expect((await call("/v1/friends/alice/accept", "bob", "POST")).status).toBe(
    200,
  );
}
async function computer() {
  const r = await call("/v1/computers", "alice", "POST", {
    name: "Desktop",
    platform: "windows",
  });
  expect(r.status).toBe(201);
  return (await r.json()).id as string;
}
async function invitation(host: string) {
  const r = await call("/v1/requests", "bob", "POST", {
    computer: host,
    permissions: ["view", "audio", "gamepad"],
  });
  expect(r.status).toBe(201);
  return (await r.json()).id as string;
}
beforeEach(async () => {
  db = new Database();
  const { privateKey } = await generateKeyPair("ES256", { extractable: true });
  env = {
    DB: db as unknown as D1Database,
    PUBLIC_ORIGIN: base,
    GITHUB_CLIENT_ID: "test-client",
    GITHUB_CLIENT_SECRET: "test-secret",
    GRANT_PRIVATE_JWK: JSON.stringify(await exportJWK(privateKey)),
  };
  for (const [id, token] of Object.entries(tokens)) {
    await db
      .prepare("INSERT INTO accounts(id,github_id,handle,name) VALUES(?,?,?,?)")
      .bind(id, id, id, id)
      .run();
    await db
      .prepare("INSERT INTO sessions(hash,user_id,expires) VALUES(?,?,?)")
      .bind(await hash(token), id, Math.floor(Date.now() / 1000) + 3600)
      .run();
  }
});
describe("authentication", () => {
  it("requires authentication and origin protection for browser mutations", async () => {
    expect((await call("/v1/friends", null)).status).toBe(401);
    const request = new Request(base + "/v1/friends/bob", {
      method: "PUT",
      headers: {
        cookie: `__Host-secpar-session=${tokens.alice}`,
        origin: "https://evil.example",
      },
    });
    expect((await handle(request, env)).status).toBe(403);
    expect((await call("/auth/logout", "alice", "POST")).status).toBe(204);
    expect((await call("/v1/me")).status).toBe(401);
  });
  it("uses browser-bound one-use OAuth state and PKCE and stores only session hashes", async () => {
    const begin = await handle(new Request(base + "/auth/start"), env);
    const location = new URL(begin.headers.get("location")!);
    const state = location.searchParams.get("state")!;
    expect(location.searchParams.get("code_challenge_method")).toBe("S256");
    expect(location.searchParams.get("code_challenge")).toHaveLength(43);
    let verified = false;
    const provider: typeof fetch = async (input, init) => {
      if (String(input).includes("access_token")) {
        const body = JSON.parse(String(init?.body));
        expect(body.code_verifier).toHaveLength(64);
        expect(body.redirect_uri).toBe(base + "/auth/callback");
        verified = true;
        return Response.json({ access_token: "provider-only-secret" });
      }
      return Response.json({
        id: 12345,
        login: "newfriend",
        name: "New Friend",
      });
    };
    const callback = new Request(
      base + `/auth/callback?code=provider-code&state=${state}`,
      { headers: { cookie: `__Host-secpar-oauth=${state}` } },
    );
    expect(
      (await handle(new Request(callback.url), env, provider)).status,
    ).toBe(400);
    const result = await handle(callback, env, provider);
    expect(result.status).toBe(302);
    expect(verified).toBe(true);
    const cookie = result.headers
      .getSetCookie()
      .find((s) => s.startsWith("__Host-secpar-session="))!;
    expect(cookie).toContain("HttpOnly");
    expect(cookie).toContain("Secure");
    const token = cookie.split(";")[0].split("=")[1];
    expect(
      await db
        .prepare("SELECT 1 FROM sessions WHERE hash=?")
        .bind(token)
        .first(),
    ).toBeNull();
    const me = await handle(
      new Request(base + "/v1/me", {
        headers: { cookie: cookie.split(";")[0] },
      }),
      env,
    );
    expect((await me.json()).handle).toBe("newfriend");
    expect(
      (
        await handle(
          new Request(callback.url, {
            headers: { cookie: `__Host-secpar-oauth=${state}` },
          }),
          env,
          provider,
        )
      ).status,
    ).toBe(400);
  });
});
describe("friend and guest boundaries", () => {
  it("a friend request grants no access and cannot be self-accepted", async () => {
    const host = await computer();
    expect((await call("/v1/friends/bob", "alice", "PUT")).status).toBe(202);
    expect((await call("/v1/friends/bob/accept", "alice", "POST")).status).toBe(
      404,
    );
    expect(
      (
        await call("/v1/requests", "bob", "POST", {
          computer: host,
          permissions: ["view"],
        })
      ).status,
    ).toBe(403);
  });
  it("only the owner approves and only the intended guest claims one limited signed grant", async () => {
    await friendship();
    const host = await computer();
    const id = await invitation(host);
    expect(
      (
        await call(`/v1/requests/${id}/approve`, "bob", "POST", {
          permissions: ["view"],
        })
      ).status,
    ).toBe(403);
    expect(
      (await call(`/v1/requests/${id}/claim`, "mallory", "POST")).status,
    ).toBe(404);
    expect(
      (
        await call(`/v1/requests/${id}/approve`, "alice", "POST", {
          permissions: ["view", "keyboard"],
        })
      ).status,
    ).toBe(400);
    expect(
      (
        await call(`/v1/requests/${id}/approve`, "alice", "POST", {
          permissions: ["view", "gamepad"],
        })
      ).status,
    ).toBe(200);
    const claims = await Promise.all([
      call(`/v1/requests/${id}/claim`, "bob", "POST"),
      call(`/v1/requests/${id}/claim`, "bob", "POST"),
    ]);
    expect(claims.map((r) => r.status).sort()).toEqual([200, 409]);
    const token = (await claims.find((r) => r.status === 200)!.json()).grant;
    const keys = await (
      await handle(new Request(base + "/.well-known/jwks.json"), env)
    ).json();
    expect(keys.keys[0].d).toBeUndefined();
    const { payload } = await jwtVerify(
      token,
      await importJWK(keys.keys[0], "ES256"),
      { issuer: base, audience: host },
    );
    expect(payload.sub).toBe("bob");
    expect(payload.jti).toBe(id);
    expect(payload.permissions).toEqual(["gamepad", "view"]);
    expect(payload.exp! - payload.iat!).toBeLessThanOrEqual(60);
    await expect(
      jwtVerify(token, await importJWK(keys.keys[0], "ES256"), {
        issuer: base,
        audience: "different-host",
      }),
    ).rejects.toThrow();
  });
  it("blocking revokes claimed access and prevents new invitations in both directions", async () => {
    await friendship();
    const host = await computer();
    const id = await invitation(host);
    await call(`/v1/requests/${id}/approve`, "alice", "POST", {
      permissions: ["view"],
    });
    await call(`/v1/requests/${id}/claim`, "bob", "POST");
    expect(
      (await (await call(`/v1/requests/${id}`, "bob")).json()).active,
    ).toBe(true);
    await call("/v1/blocks/alice", "bob", "PUT");
    expect(
      (await (await call(`/v1/requests/${id}`, "alice")).json()).active,
    ).toBe(false);
    expect((await call("/v1/friends/alice", "bob", "PUT")).status).toBe(404);
    expect((await call("/v1/friends/bob", "alice", "PUT")).status).toBe(404);
    expect(
      (
        await call("/v1/requests", "bob", "POST", {
          computer: host,
          permissions: ["view"],
        })
      ).status,
    ).toBe(403);
  });
  it("does not issue admin permissions or expired grants", async () => {
    await friendship();
    const host = await computer();
    expect(
      (
        await call("/v1/requests", "bob", "POST", {
          computer: host,
          permissions: ["unlock"],
        })
      ).status,
    ).toBe(400);
    const id = await invitation(host);
    await db
      .prepare("UPDATE connection_requests SET expires=0 WHERE id=?")
      .bind(id)
      .run();
    expect(
      (
        await call(`/v1/requests/${id}/approve`, "alice", "POST", {
          permissions: ["view"],
        })
      ).status,
    ).toBe(409);
    expect((await call(`/v1/requests/${id}/claim`, "bob", "POST")).status).toBe(
      403,
    );
  });
});

describe("native device sign-in", () => {
  it("keeps polling credentials off the browser and exchanges an approved code once", async () => {
    const start = await call("/auth/device", null, "POST", {
      name: "Windows desktop",
    });
    expect(start.status).toBe(201);
    const device = await start.json();
    expect(device.verification_uri_complete).not.toContain(device.device_code);
    const login = await handle(
      new Request(device.verification_uri_complete),
      env,
    );
    expect(login.status).toBe(302);
    expect(login.headers.get("location")).toContain(
      "/auth/start?activate_code=",
    );
    expect(
      (
        await call("/auth/device/token", null, "POST", {
          device_code: device.device_code,
        })
      ).status,
    ).toBe(400);
    expect(
      (await call("/activate", "alice", "POST", { code: device.user_code }))
        .status,
    ).toBe(200);
    expect(
      (await call("/activate", "bob", "POST", { code: device.user_code }))
        .status,
    ).toBe(409);
    await db.prepare("UPDATE device_auth SET poll_after=0").run();
    const tokenResult = await call("/auth/device/token", null, "POST", {
      device_code: device.device_code,
    });
    expect(tokenResult.status).toBe(200);
    const token = (await tokenResult.json()).access_token;
    const me = await handle(
      new Request(base + "/v1/me", {
        headers: { authorization: `Bearer ${token}` },
      }),
      env,
    );
    expect((await me.json()).id).toBe("alice");
    expect(
      (
        await call("/auth/device/token", null, "POST", {
          device_code: device.device_code,
        })
      ).status,
    ).toBe(400);
  });
});

describe("configuration and resource limits", () => {
  it("does not consume an approved claim while signing is unconfigured", async () => {
    await friendship();
    const host = await computer();
    const id = await invitation(host);
    await call(`/v1/requests/${id}/approve`, "alice", "POST", {
      permissions: ["view"],
    });
    const key = env.GRANT_PRIVATE_JWK;
    env.GRANT_PRIVATE_JWK = "";
    expect((await call(`/v1/requests/${id}/claim`, "bob", "POST")).status).toBe(
      503,
    );
    env.GRANT_PRIVATE_JWK = key;
    expect((await call(`/v1/requests/${id}/claim`, "bob", "POST")).status).toBe(
      200,
    );
  });
  it("limits unauthenticated login creation and bounds streamed request bodies", async () => {
    for (let n = 0; n < 10; n++)
      expect(
        (await handle(new Request(base + "/auth/start"), env)).status,
      ).toBe(302);
    expect((await handle(new Request(base + "/auth/start"), env)).status).toBe(
      429,
    );
    const request = new Request(base + "/v1/computers", {
      method: "POST",
      headers: {
        authorization: `Bearer ${tokens.alice}`,
        "content-type": "application/json",
        "content-length": "1",
      },
      body: JSON.stringify({ name: "x".repeat(9000), platform: "windows" }),
    });
    expect((await handle(request, env)).status).toBe(413);
  });
});

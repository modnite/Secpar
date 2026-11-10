// What a host owner allows each friend. Shared by the signed-in routes and the host's own routes.
// `keyboard` and `mouse` are the fine setting. A row without them follows `input`, so an older client or host
// that only knows view or full keeps working: `input` stays "full" only while both are on.
interface Environment { DB: D1Database }
const q = (env: Environment, sql: string, ...args: (string | number)[]) => env.DB.prepare(sql).bind(...args);

/** Columns for a query that joins friend_settings as `s`. Add after `COALESCE(s.connect,'ask') AS connect`. */
export const EFFECTIVE =
  "COALESCE(s.keyboard,CASE WHEN COALESCE(s.input,'view')='full' THEN 1 ELSE 0 END) AS keyboard,COALESCE(s.mouse,CASE WHEN COALESCE(s.input,'view')='full' THEN 1 ELSE 0 END) AS mouse";

/** D1 returns 0 and 1. Clients and hosts get real booleans. */
export function flags<T extends { keyboard?: unknown; mouse?: unknown }>(rows: T[]) {
  return rows.map((r) => ({ ...r, keyboard: !!r.keyboard, mouse: !!r.mouse }));
}

export async function friendSettings(env: Environment, owner: string, friend: string) {
  const row = await q(
    env,
    `SELECT COALESCE(s.input,'view') AS input,COALESCE(s.connect,'ask') AS connect,${EFFECTIVE} FROM (SELECT 1) LEFT JOIN friend_settings s ON s.owner=? AND s.friend=?`,
    owner,
    friend,
  ).first<{ input: string; connect: string; keyboard: number; mouse: number }>();
  return { input: row?.input ?? "view", connect: row?.connect ?? "ask", keyboard: !!row?.keyboard, mouse: !!row?.mouse };
}

type Body = Record<string, unknown>;
const absent = (v: unknown) => v === undefined || v === null;

/** Validates and stores a change. Returns false for an invalid body. Running sessions follow at the next host check. */
export async function saveSettings(env: Environment, owner: string, friend: string, b: Body): Promise<boolean> {
  if (!absent(b.input) && b.input !== "view" && b.input !== "full") return false;
  if (!absent(b.connect) && b.connect !== "ask" && b.connect !== "always") return false;
  if (!absent(b.keyboard) && typeof b.keyboard !== "boolean") return false;
  if (!absent(b.mouse) && typeof b.mouse !== "boolean") return false;
  const input = absent(b.input) ? null : (b.input as string), connect = absent(b.connect) ? null : (b.connect as string);
  const keyboard = absent(b.keyboard) ? null : (b.keyboard as boolean), mouse = absent(b.mouse) ? null : (b.mouse as boolean);
  if (input === null && connect === null && keyboard === null && mouse === null) return false;
  await q(
    env,
    "INSERT INTO friend_settings(owner,friend,input,connect) VALUES(?,?,COALESCE(NULLIF(?,''),'view'),COALESCE(NULLIF(?,''),'ask')) ON CONFLICT(owner,friend) DO UPDATE SET input=COALESCE(NULLIF(?,''),input),connect=COALESCE(NULLIF(?,''),connect)",
    owner,
    friend,
    input ?? "",
    connect ?? "",
    input ?? "",
    connect ?? "",
  ).run();
  if (keyboard !== null || mouse !== null) {
    await q(
      env,
      "UPDATE friend_settings SET keyboard=COALESCE(NULLIF(?,-1),keyboard,CASE WHEN input='full' THEN 1 ELSE 0 END),mouse=COALESCE(NULLIF(?,-1),mouse,CASE WHEN input='full' THEN 1 ELSE 0 END) WHERE owner=? AND friend=?",
      keyboard === null ? -1 : keyboard ? 1 : 0,
      mouse === null ? -1 : mouse ? 1 : 0,
      owner,
      friend,
    ).run();
    await q(env, "UPDATE friend_settings SET input=CASE WHEN keyboard=1 AND mouse=1 THEN 'full' ELSE 'view' END WHERE owner=? AND friend=?", owner, friend).run();
  } else if (input !== null) {
    // The single setting was chosen again. It decides both.
    await q(env, "UPDATE friend_settings SET keyboard=NULL,mouse=NULL WHERE owner=? AND friend=?", owner, friend).run();
  }
  const now = await friendSettings(env, owner, friend);
  if (!now.keyboard && !now.mouse) await revokeInputGrants(env, owner, friend);
  return true;
}

/** View only ends any grant that already carries input. Lowering one of the two is handled by the host at its next check. */
export async function revokeInputGrants(env: Environment, owner: string, friend: string) {
  await q(
    env,
    "UPDATE connection_requests SET state='revoked' WHERE owner=? AND guest=? AND state IN ('pending','approved','claimed') AND (requested LIKE '%\"keyboard\"%' OR requested LIKE '%\"mouse\"%') AND (COALESCE(approved,requested) LIKE '%\"keyboard\"%' OR COALESCE(approved,requested) LIKE '%\"mouse\"%')",
    owner,
    friend,
  ).run();
}

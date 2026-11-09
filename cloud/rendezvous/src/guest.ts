// A friend reaches the host through the relay without a paired-device ticket. The relay cannot
// check the signed grant, so it only limits what an unauthenticated socket may do. The host
// verifies the grant and answers with `welcome`, which is when the socket stops being limited.

/** Unauthenticated guest sockets per PC. */
export const MAX_PENDING_GUESTS = 2;
/** Open guest sockets per PC: three friends and two still waiting to be accepted. */
export const MAX_GUESTS = 5;
/** Seconds a socket may stay unauthenticated. */
export const PENDING_SECONDS = 10;
/** Largest first frame. A signed grant is under 4096 characters, so this is generous. */
export const MAX_FIRST_FRAME = 5000;

/** The only frame an unauthenticated guest may send: `{"t":"guest","grant":"..."}`. */
export function firstFrameOk(msg: string | ArrayBuffer): boolean {
  if (typeof msg !== "string" || msg.length > MAX_FIRST_FRAME) return false;
  try {
    const m = JSON.parse(msg) as { t?: unknown; grant?: unknown };
    return m.t === "guest" && typeof m.grant === "string" && m.grant.length <= 4096;
  } catch {
    return false;
  }
}

/** True for the host's `welcome`, the first sign that it accepted the grant. */
export function isWelcome(msg: string): boolean {
  try {
    return (JSON.parse(msg) as { t?: unknown }).t === "welcome";
  } catch {
    return false;
  }
}

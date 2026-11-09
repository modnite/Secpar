import { describe, expect, it } from "vitest";
import { firstFrameOk, isWelcome, MAX_FIRST_FRAME } from "../src/guest";

describe("guest sockets", () => {
  it("lets an unauthenticated friend send only the guest frame", () => {
    expect(firstFrameOk(JSON.stringify({ t: "guest", grant: "a.b.c" }))).toBe(true);
    for (const bad of [
      JSON.stringify({ t: "hello", token: "x", v: 1, device: "x" }),
      JSON.stringify({ t: "key", vk: 65, down: true }),
      JSON.stringify({ t: "guest" }),
      JSON.stringify({ t: "guest", grant: 5 }),
      JSON.stringify({ t: "guest", grant: "x".repeat(4097) }),
      "x".repeat(MAX_FIRST_FRAME + 1),
      "not json",
      new ArrayBuffer(8),
    ]) {
      expect(firstFrameOk(bad as string | ArrayBuffer)).toBe(false);
    }
  });
  it("treats only the host's welcome as acceptance", () => {
    expect(isWelcome(JSON.stringify({ t: "welcome", v: 1 }))).toBe(true);
    expect(isWelcome(JSON.stringify({ t: "error", code: "guest_refused" }))).toBe(false);
    expect(isWelcome("{")).toBe(false);
  });
});

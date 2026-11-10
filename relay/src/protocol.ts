// CCS/1 M:relay/protocol | PURPOSE:PROTOCOL.md §13 constants shared by the Worker and its tests
// SEC:the relay holds no device keys; it verifies who connects, never what they send.

/** Data frames above this are a protocol violation (QUIC packets here are ≤ 1452 B). */
export const MAX_FRAME = 2048;
/** An unauthenticated socket is closed after this. */
export const AUTH_TIMEOUT_MS = 10_000;
/** Unauthenticated sockets a pair holds at once; a new one evicts the oldest (§13.1). Authenticated
 * sockets are never counted: at most one per device, since a newer one replaces the older. */
export const MAX_PENDING_PER_PAIR = 4;

export const PATH = "/v1/relay";

export const enum Close {
  /** Bad or missing signature, or none in time. */
  Auth = 4001,
  /** A frame §13 does not allow (data before auth, unknown text). */
  Protocol = 4002,
  /** The same device connected again; the newer socket wins. */
  Replaced = 4003,
  /** An unauthenticated socket pushed out by a newer one (the pending pool was full). */
  Evicted = 4004,
  /** A data frame over MAX_FRAME. */
  TooBig = 1009,
}

const encoder = new TextEncoder();
const PAIR_CONTEXT = encoder.encode("wc-relay-v1 pair");
const AUTH_CONTEXT = encoder.encode("wc-relay-v1 auth");

/** 64 lowercase hex digits → 32 bytes; anything else is null. */
export function key(hex: string | null): Uint8Array | null {
  if (hex === null || !/^[0-9a-f]{64}$/.test(hex)) return null;
  return fromHex(hex);
}

export function fromHex(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function concat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const part of parts) {
    out.set(part, at);
    at += part.length;
  }
  return out;
}

function compare(a: Uint8Array, b: Uint8Array): number {
  for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return a[i] - b[i];
  return 0;
}

/** The pair's rendezvous name: SHA-256("wc-relay-v1 pair" ‖ lower key ‖ higher key), hex. */
export async function pairName(a: Uint8Array, b: Uint8Array): Promise<string> {
  const [low, high] = compare(a, b) < 0 ? [a, b] : [b, a];
  return toHex(new Uint8Array(await crypto.subtle.digest("SHA-256", concat(PAIR_CONTEXT, low, high))));
}

/** What a device signs: "wc-relay-v1 auth" ‖ nonce ‖ own key ‖ peer key. */
export function authMessage(nonce: Uint8Array, me: Uint8Array, peer: Uint8Array): Uint8Array {
  return concat(AUTH_CONTEXT, nonce, me, peer);
}

/** Ed25519 over [authMessage]; false for a malformed key or signature. */
export async function verifyAuth(
  me: Uint8Array,
  peer: Uint8Array,
  nonce: Uint8Array,
  signature: Uint8Array,
): Promise<boolean> {
  if (signature.length !== 64) return false;
  try {
    const publicKey = await crypto.subtle.importKey("raw", me, { name: "Ed25519" }, false, ["verify"]);
    return await crypto.subtle.verify("Ed25519", publicKey, signature, authMessage(nonce, me, peer));
  } catch {
    return false;
  }
}

// CCS/1 M:relay | FLOW:device WebSocket -> Worker (shape, IP limit) -> RelayPair DO (auth, forward)
// SEC:payloads are QUIC packets encrypted end to end (§2); the relay never stores or logs them.
// DEC:rendezvous needs both devices to name each other, so nobody reaches a device that did not ask.

import { DurableObject } from "cloudflare:workers";
import {
  AUTH_TIMEOUT_MS,
  Close,
  MAX_FRAME,
  MAX_PENDING_PER_PAIR,
  PATH,
  fromHex,
  key,
  pairName,
  toHex,
  verifyAuth,
} from "./protocol";

export interface Env {
  PAIRS: DurableObjectNamespace<RelayPair>;
  CONNECT_LIMIT?: RateLimit;
  RATE_BYTES_PER_SEC?: string;
  BURST_BYTES?: string;
  /** Tests only; §13 fixes 10 s. */
  AUTH_TIMEOUT_MS?: string;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.pathname !== PATH) {
      return new Response("WindConnect relay (PROTOCOL.md §13)\n", { status: url.pathname === "/" ? 200 : 404 });
    }
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("expected a WebSocket upgrade\n", { status: 426 });
    }
    const me = key(url.searchParams.get("me"));
    const peer = key(url.searchParams.get("peer"));
    if (me === null || peer === null || toHex(me) === toHex(peer)) {
      return new Response("me and peer must be two different 64-hex keys\n", { status: 400 });
    }
    if (env.CONNECT_LIMIT) {
      const ip = request.headers.get("CF-Connecting-IP") ?? "unknown";
      const { success } = await env.CONNECT_LIMIT.limit({ key: ip });
      if (!success) return new Response("too many connections\n", { status: 429 });
    }
    const stub = env.PAIRS.get(env.PAIRS.idFromName(await pairName(me, peer)));
    return stub.fetch(request);
  },
} satisfies ExportedHandler<Env>;

interface Attachment {
  me: string;
  peer: string;
  nonce: string;
  authed: boolean;
  deadline: number;
}

interface Bucket {
  tokens: number;
  at: number;
}

/** One paired couple's rendezvous: at most one authenticated socket per device. */
export class RelayPair extends DurableObject<Env> {
  /** Per device key; rebuilt after hibernation (a fresh, full bucket). */
  private buckets = new Map<string, Bucket>();

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    // Keep-alive without waking the object.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('{"t":"ping"}', '{"t":"pong"}'));
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const me = url.searchParams.get("me")!;
    const peer = url.searchParams.get("peer")!;
    // CCS/1 SEC:unsigned sockets never lock a pair out: the oldest makes room, signed ones never count.
    const pending = this.ctx
      .getWebSockets()
      .filter((ws) => !(ws.deserializeAttachment() as Attachment).authed)
      .sort((a, b) => (a.deserializeAttachment() as Attachment).deadline - (b.deserializeAttachment() as Attachment).deadline);
    for (const old of pending.slice(0, Math.max(0, pending.length - MAX_PENDING_PER_PAIR + 1))) {
      old.close(Close.Evicted, "too many unauthenticated connections");
    }
    const [client, server] = Object.values(new WebSocketPair());
    const nonce = toHex(crypto.getRandomValues(new Uint8Array(32)));
    const deadline = Date.now() + Number(this.env.AUTH_TIMEOUT_MS ?? AUTH_TIMEOUT_MS);
    this.ctx.acceptWebSocket(server, [me]);
    server.serializeAttachment({ me, peer, nonce, authed: false, deadline } satisfies Attachment);
    server.send(JSON.stringify({ t: "challenge", nonce }));
    const alarm = await this.ctx.storage.getAlarm();
    if (alarm === null || alarm > deadline) await this.ctx.storage.setAlarm(deadline);
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): Promise<void> {
    const state = ws.deserializeAttachment() as Attachment;
    if (!state.authed) return this.authenticate(ws, state, message);
    if (typeof message === "string") return ws.close(Close.Protocol, "only data frames after auth");
    if (message.byteLength > MAX_FRAME) return ws.close(Close.TooBig, "frame too large");
    // CCS/1 RULE:over the rate the frame is dropped, like a lost UDP packet; QUIC recovers.
    if (!this.take(state.me, message.byteLength)) return;
    const other = this.authed(state.peer);
    if (other) other.send(message);
  }

  private async authenticate(ws: WebSocket, state: Attachment, message: string | ArrayBuffer): Promise<void> {
    if (typeof message !== "string") return ws.close(Close.Protocol, "authenticate first");
    let sig: unknown = null;
    try {
      const frame = JSON.parse(message);
      if (frame?.t === "auth") sig = frame.sig;
    } catch {
      // Not JSON: fails below.
    }
    const ok =
      Date.now() <= state.deadline &&
      typeof sig === "string" &&
      /^[0-9a-f]{128}$/.test(sig) &&
      (await verifyAuth(fromHex(state.me), fromHex(state.peer), fromHex(state.nonce), fromHex(sig)));
    if (!ok) return ws.close(Close.Auth, "authentication failed");
    const previous = this.authed(state.me);
    state.authed = true;
    ws.serializeAttachment(state);
    if (previous) previous.close(Close.Replaced, "replaced by a newer connection");
    const other = this.authed(state.peer);
    ws.send(JSON.stringify({ t: "ready", peer: other !== undefined }));
    if (other && !previous) other.send(JSON.stringify({ t: "peer", present: true }));
  }

  async webSocketClose(ws: WebSocket): Promise<void> {
    this.gone(ws);
  }

  async webSocketError(ws: WebSocket): Promise<void> {
    this.gone(ws);
  }

  /** Closes sockets that did not authenticate in time. */
  async alarm(): Promise<void> {
    const now = Date.now();
    let next = Infinity;
    for (const ws of this.ctx.getWebSockets()) {
      const state = ws.deserializeAttachment() as Attachment;
      if (state.authed) continue;
      if (state.deadline <= now) ws.close(Close.Auth, "authentication timed out");
      else next = Math.min(next, state.deadline);
    }
    if (next !== Infinity) await this.ctx.storage.setAlarm(next);
  }

  private gone(ws: WebSocket): void {
    const state = ws.deserializeAttachment() as Attachment;
    if (!state.authed || this.authed(state.me, ws)) return;
    this.buckets.delete(state.me);
    this.authed(state.peer)?.send(JSON.stringify({ t: "peer", present: false }));
  }

  /** The authenticated, open socket of device `hex`, other than `except`. */
  private authed(hex: string, except?: WebSocket): WebSocket | undefined {
    return this.ctx.getWebSockets(hex).find((ws) => {
      if (ws === except || ws.readyState !== WebSocket.READY_STATE_OPEN) return false;
      return (ws.deserializeAttachment() as Attachment).authed;
    });
  }

  /** Token bucket per device: RATE_BYTES_PER_SEC, bursting to BURST_BYTES. */
  private take(device: string, bytes: number): boolean {
    const rate = Number(this.env.RATE_BYTES_PER_SEC ?? 1_048_576);
    const burst = Number(this.env.BURST_BYTES ?? 4_194_304);
    const now = Date.now();
    const bucket = this.buckets.get(device) ?? { tokens: burst, at: now };
    bucket.tokens = Math.min(burst, bucket.tokens + ((now - bucket.at) / 1000) * rate);
    bucket.at = now;
    this.buckets.set(device, bucket);
    if (bucket.tokens < bytes) return false;
    bucket.tokens -= bytes;
    return true;
  }
}

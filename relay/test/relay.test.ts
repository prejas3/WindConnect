// WN-59: the relay's rules (PROTOCOL.md §13), run in the Workers runtime.
import { SELF, env, runDurableObjectAlarm } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import { authMessage, fromHex, pairName, toHex } from "../src/protocol";

interface Device {
  hex: string;
  sign(data: Uint8Array): Promise<Uint8Array>;
}

async function device(): Promise<Device> {
  const pair = (await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"])) as CryptoKeyPair;
  const raw = new Uint8Array((await crypto.subtle.exportKey("raw", pair.publicKey)) as ArrayBuffer);
  return {
    hex: toHex(raw),
    sign: async (data) => new Uint8Array(await crypto.subtle.sign("Ed25519", pair.privateKey, data)),
  };
}

type Frame = string | ArrayBuffer;

interface Socket {
  ws: WebSocket;
  next(): Promise<Frame>;
  closed: Promise<number>;
  received: Frame[];
}

const upgrade = { headers: { Upgrade: "websocket" } };

async function open(me: string, peer: string): Promise<Socket> {
  const response = await SELF.fetch(`https://relay.test/v1/relay?me=${me}&peer=${peer}`, upgrade);
  expect(response.status).toBe(101);
  const ws = response.webSocket!;
  const received: Frame[] = [];
  const waiting: ((frame: Frame) => void)[] = [];
  ws.addEventListener("message", (event: MessageEvent) => {
    const frame = event.data as Frame;
    const take = waiting.shift();
    if (take) take(frame);
    else received.push(frame);
  });
  const closed = new Promise<number>((resolve) => ws.addEventListener("close", (event: CloseEvent) => resolve(event.code)));
  ws.accept();
  return {
    ws,
    received,
    closed,
    next: () =>
      received.length > 0
        ? Promise.resolve(received.shift()!)
        : new Promise((resolve, reject) => {
            waiting.push(resolve);
            setTimeout(() => reject(new Error("no frame within 2 s")), 2000);
          }),
  };
}

const json = async (socket: Socket) => JSON.parse((await socket.next()) as string);
/** Binary frames reach this test client as Blob. */
const bytes = async (socket: Socket) => {
  const frame = (await socket.next()) as unknown as Blob | ArrayBuffer;
  return new Uint8Array(frame instanceof Blob ? await frame.arrayBuffer() : frame);
};

/** Connects and authenticates `me` towards `peer`. */
async function join(me: Device, peer: Device): Promise<{ socket: Socket; ready: { peer: boolean } }> {
  const socket = await open(me.hex, peer.hex);
  const challenge = await json(socket);
  expect(challenge.t).toBe("challenge");
  const sig = await me.sign(authMessage(fromHex(challenge.nonce), fromHex(me.hex), fromHex(peer.hex)));
  socket.ws.send(JSON.stringify({ t: "auth", sig: toHex(sig) }));
  const ready = await json(socket);
  expect(ready.t).toBe("ready");
  return { socket, ready };
}

const quiet = () => new Promise((resolve) => setTimeout(resolve, 150));

describe("shape of requests", () => {
  it("answers only WebSocket upgrades with two distinct keys on /v1/relay", async () => {
    const [a, b] = [await device(), await device()];
    expect((await SELF.fetch("https://relay.test/")).status).toBe(200);
    expect((await SELF.fetch("https://relay.test/other")).status).toBe(404);
    expect((await SELF.fetch(`https://relay.test/v1/relay?me=${a.hex}&peer=${b.hex}`)).status).toBe(426);
    for (const query of [
      `me=${a.hex}`,
      `me=${a.hex}&peer=${a.hex}`,
      `me=${a.hex.toUpperCase()}&peer=${b.hex}`,
      `me=${a.hex.slice(2)}&peer=${b.hex}`,
      `me=zz${a.hex.slice(2)}&peer=${b.hex}`,
    ]) {
      expect((await SELF.fetch(`https://relay.test/v1/relay?${query}`, upgrade)).status, query).toBe(400);
    }
  });
});

describe("rendezvous", () => {
  it("forwards data frames byte for byte between the two devices", async () => {
    const [a, b] = [await device(), await device()];
    const alice = await join(a, b);
    expect(alice.ready.peer).toBe(false);
    const bob = await join(b, a);
    expect(bob.ready.peer).toBe(true);
    expect(await json(alice.socket)).toEqual({ t: "peer", present: true });

    const packet = crypto.getRandomValues(new Uint8Array(1452));
    alice.socket.ws.send(packet);
    expect(await bytes(bob.socket)).toEqual(packet);
    const reply = crypto.getRandomValues(new Uint8Array(40));
    bob.socket.ws.send(reply);
    expect(await bytes(alice.socket)).toEqual(reply);

    bob.socket.ws.close(1000);
    expect(await json(alice.socket)).toEqual({ t: "peer", present: false });
  });

  it("does not let a third device reach either of them", async () => {
    const [a, b, mallory] = [await device(), await device(), await device()];
    const alice = await join(a, b);
    const bob = await join(b, a);
    await alice.socket.next(); // peer present
    // Mallory names Bob, but Bob named only Alice: another rendezvous.
    const intruder = await join(mallory, b);
    expect(intruder.ready.peer).toBe(false);
    intruder.socket.ws.send(new Uint8Array([1, 2, 3]));
    await quiet();
    expect(bob.socket.received).toEqual([]);
    expect(alice.socket.received).toEqual([]);
  });

  it("refuses a forged, replayed or misdirected signature and data before authentication", async () => {
    const [a, b, mallory] = [await device(), await device(), await device()];
    const sign = async (signer: Device, socket: Socket, peer: Device, nonce?: string) => {
      const challenge = await json(socket);
      const sig = await signer.sign(authMessage(fromHex(nonce ?? challenge.nonce), fromHex(a.hex), fromHex(peer.hex)));
      return { nonce: challenge.nonce as string, sig: toHex(sig) };
    };

    // Mallory claims Alice's key but signs with her own.
    const forged = await open(a.hex, b.hex);
    forged.ws.send(JSON.stringify({ t: "auth", sig: (await sign(mallory, forged, b)).sig }));
    expect(await forged.closed).toBe(4001);

    // A signature for another connection's nonce is useless.
    const first = await open(a.hex, b.hex);
    const { nonce } = await sign(a, first, b);
    const second = await open(a.hex, b.hex);
    second.ws.send(JSON.stringify({ t: "auth", sig: (await sign(a, second, b, nonce)).sig }));
    expect(await second.closed).toBe(4001);

    // Signed for another peer.
    const third = await open(a.hex, b.hex);
    third.ws.send(JSON.stringify({ t: "auth", sig: (await sign(a, third, mallory)).sig }));
    expect(await third.closed).toBe(4001);

    const early = await open(a.hex, b.hex);
    await early.next();
    early.ws.send(new Uint8Array([0]));
    expect(await early.closed).toBe(4002);

    const garbage = await open(a.hex, b.hex);
    await garbage.next();
    garbage.ws.send("not json");
    expect(await garbage.closed).toBe(4001);
    first.ws.close();
  });

  it("closes a socket that does not authenticate in time", async () => {
    const [a, b] = [await device(), await device()];
    const idle = await open(a.hex, b.hex);
    await idle.next();
    await new Promise((resolve) => setTimeout(resolve, 400));
    await runDurableObjectAlarm(env.PAIRS.get(env.PAIRS.idFromName(await pairName(fromHex(a.hex), fromHex(b.hex)))));
    expect(await idle.closed).toBe(4001);
  });

  it("keeps one connection per device: the newer replaces the older", async () => {
    const [a, b] = [await device(), await device()];
    const bob = await join(b, a);
    const old = await join(a, b);
    expect(await json(bob.socket)).toEqual({ t: "peer", present: true });
    const fresh = await join(a, b);
    expect(fresh.ready.peer).toBe(true);
    expect(await old.socket.closed).toBe(4003);
    await quiet();
    expect(bob.socket.received, "Bob never saw Alice leave").toEqual([]);
    fresh.socket.ws.send(new Uint8Array([7]));
    expect(await bytes(bob.socket)).toEqual(new Uint8Array([7]));
  });
});

describe("limits", () => {
  it("closes on an oversized frame or text after authentication, and answers pings", async () => {
    const [a, b] = [await device(), await device()];
    const big = await join(a, b);
    big.socket.ws.send(new Uint8Array(2049));
    expect(await big.socket.closed).toBe(1009);

    const chatty = await join(a, b);
    chatty.socket.ws.send('{"t":"ping"}');
    expect(await json(chatty.socket)).toEqual({ t: "pong" });
    chatty.socket.ws.send('{"t":"hello"}');
    expect(await chatty.socket.closed).toBe(4002);
  });

  it("drops frames over the byte rate instead of queueing them", async () => {
    const [a, b] = [await device(), await device()];
    const alice = await join(a, b);
    const bob = await join(b, a);
    await alice.socket.next();
    // Burst 8 KiB in the test config: of twenty 1 KiB frames sent at once, about eight pass.
    for (let i = 0; i < 20; i++) alice.socket.ws.send(new Uint8Array(1024).fill(i));
    await quiet();
    const passed = bob.socket.received.length;
    expect(passed).toBeGreaterThanOrEqual(8);
    expect(passed).toBeLessThan(12);
    expect(alice.socket.ws.readyState).toBe(WebSocket.READY_STATE_OPEN);
  });

  it("cannot be locked out by unsigned sockets: the oldest make room (review WN-59 r1)", async () => {
    const [a, b] = [await device(), await device()];
    // Public keys are not secret: anyone can open sockets for this pair and never sign.
    const squatters: Socket[] = [];
    for (let i = 0; i < 8; i++) squatters.push(await open(i % 2 ? a.hex : b.hex, i % 2 ? b.hex : a.hex));
    const alice = await join(a, b);
    const bob = await join(b, a);
    expect(await json(alice.socket)).toEqual({ t: "peer", present: true });
    alice.socket.ws.send(new Uint8Array([4, 2]));
    expect(await bytes(bob.socket)).toEqual(new Uint8Array([4, 2]));

    await quiet();
    const codes = await Promise.all(
      squatters.map((s) => Promise.race([s.closed, new Promise<null>((r) => setTimeout(() => r(null), 50))])),
    );
    const evicted = codes.filter((code) => code === 4004).length;
    // Squatters 5–8 push out 1–4, Alice pushes out 5, then signs; Bob fits beside 6–8.
    expect(evicted, "eight squatters and two real devices, pool of four").toBe(5);
    expect(codes.slice(0, 4).every((code) => code === 4004), "the oldest go first").toBe(true);
    for (const socket of squatters) socket.ws.close();
  });
});

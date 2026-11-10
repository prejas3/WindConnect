// WN-59: fixed vectors from crates/wind-core/src/relay.rs (keys from seeds [7; 32] and [8; 32]):
// the Worker derives the same rendezvous and accepts the core's signature.
import { describe, expect, it } from "vitest";
import { fromHex, pairName, verifyAuth } from "../src/protocol";

const alice = fromHex("ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c");
const bob = fromHex("1398f62c6d1a457c51ba6a4b5f3dbd2f69fca93216218dc8997e416bd17d93ca");
const nonce = new Uint8Array(32).fill(1);
const signature = fromHex(
  "242a37e26274de3a5a81acf4594ced30baa0b8d1a27baee55a1b27981c41235c" +
    "83965a64a828f8e0caddb58f8bfc2c3b41abb8db245dc3ff365323ff897e4a04",
);

describe("vectors shared with wind-core", () => {
  it("derives the core's pair name in either order", async () => {
    const expected = "890e36d8d7bef70886770d7b404e97b989fd1288e71d1147fef5384822b9a79f";
    expect(await pairName(alice, bob)).toBe(expected);
    expect(await pairName(bob, alice)).toBe(expected);
  });

  it("accepts the core's auth signature and nothing altered", async () => {
    expect(await verifyAuth(alice, bob, nonce, signature)).toBe(true);
    expect(await verifyAuth(bob, alice, nonce, signature)).toBe(false);
    expect(await verifyAuth(alice, bob, new Uint8Array(32).fill(2), signature)).toBe(false);
    const flipped = signature.slice();
    flipped[0] ^= 1;
    expect(await verifyAuth(alice, bob, nonce, flipped)).toBe(false);
    expect(await verifyAuth(alice, bob, nonce, signature.slice(0, 63))).toBe(false);
  });
});

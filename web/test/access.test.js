import assert from "node:assert/strict";
import { beforeEach, test } from "node:test";

import { resetAccessCache, verifyAccess } from "../src/access.js";

const TEAM = "https://team.cloudflareaccess.com";
const AUD = "aud-tag";
const env = { ACCESS_TEAM_DOMAIN: TEAM, ACCESS_AUD: AUD };
const now = Date.UTC(2026, 9, 1);

const b64url = (bytes) => Buffer.from(bytes).toString("base64url");
const keys = await crypto.subtle.generateKey(
  { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
  true,
  ["sign", "verify"],
);
const jwk = { ...(await crypto.subtle.exportKey("jwk", keys.publicKey)), kid: "k1" };
const fetcher = async (url) => {
  assert.equal(url, `${TEAM}/cdn-cgi/access/certs`);
  return new Response(JSON.stringify({ keys: [jwk] }));
};

async function token(claims, { kid = "k1", alg = "RS256" } = {}) {
  const head = b64url(JSON.stringify({ alg, kid }));
  const body = b64url(JSON.stringify(claims));
  const signature = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", keys.privateKey, new TextEncoder().encode(`${head}.${body}`));
  return `${head}.${body}.${b64url(new Uint8Array(signature))}`;
}

const valid = () => ({ aud: [AUD], iss: TEAM, exp: now / 1000 + 600, email: "owner@example.com" });
const request = (jwt, where = "header") =>
  new Request("https://kraken.example/play/", {
    headers: where === "header" ? { "Cf-Access-Jwt-Assertion": jwt } : { Cookie: `x=1; CF_Authorization=${jwt}` },
  });

beforeEach(resetAccessCache);

test("a valid assertion passes, from the header or the cookie", async () => {
  const jwt = await token(valid());
  assert.equal((await verifyAccess(request(jwt), env, { fetcher, now })).email, "owner@example.com");
  assert.ok(await verifyAccess(request(jwt, "cookie"), env, { fetcher, now }));
});

test("anything else is refused", async () => {
  const cases = [
    await token({ ...valid(), aud: ["other"] }),
    await token({ ...valid(), iss: "https://evil.cloudflareaccess.com" }),
    await token({ ...valid(), exp: now / 1000 - 1 }),
    await token(valid(), { kid: "unknown" }),
    await token(valid(), { alg: "none" }),
    "not.a.jwt",
  ];
  const tampered = (await token(valid())).split(".");
  tampered[1] = b64url(JSON.stringify({ ...valid(), email: "attacker@example.com" }));
  cases.push(tampered.join("."));
  for (const jwt of cases) assert.equal(await verifyAccess(request(jwt), env, { fetcher, now }), null, jwt);
});

test("without configuration nothing is accepted", async () => {
  const jwt = await token(valid());
  assert.equal(await verifyAccess(request(jwt), { ...env, ACCESS_AUD: "" }, { fetcher, now }), null);
  assert.equal(await verifyAccess(request(jwt), { ...env, ACCESS_TEAM_DOMAIN: "" }, { fetcher, now }), null);
  assert.equal(await verifyAccess(new Request("https://kraken.example/"), env, { fetcher, now }), null);
});

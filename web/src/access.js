// Cloudflare Access check for /play/*. Access itself sits in front of the
// Worker, but the Worker still verifies the signed assertion it forwards, so
// a missing or misconfigured Access application never opens the proxy.

const CACHE_MS = 10 * 60 * 1000;
let cached = { url: "", at: 0, keys: null };

function b64url(text) {
  const base = text.replace(/-/g, "+").replace(/_/g, "/");
  const padded = base + "=".repeat((4 - (base.length % 4)) % 4);
  return Uint8Array.from(atob(padded), (c) => c.charCodeAt(0));
}

const decodeJson = (part) => JSON.parse(new TextDecoder().decode(b64url(part)));

async function keys(url, fetcher, now) {
  if (cached.url === url && cached.keys && now - cached.at < CACHE_MS) return cached.keys;
  const response = await fetcher(url);
  if (!response.ok) throw new Error(`access certs: ${response.status}`);
  const { keys: list } = await response.json();
  cached = { url, at: now, keys: list || [] };
  return cached.keys;
}

function cookie(request, name) {
  for (const part of (request.headers.get("Cookie") || "").split(";")) {
    const [key, ...value] = part.trim().split("=");
    if (key === name) return value.join("=");
  }
  return null;
}

/// The verified JWT claims, or `null`. `env.ACCESS_TEAM_DOMAIN` is the
/// team's https origin and `env.ACCESS_AUD` the application's audience tag;
/// with either unset nothing is accepted.
export async function verifyAccess(request, env, { fetcher = fetch, now = Date.now() } = {}) {
  const team = String(env.ACCESS_TEAM_DOMAIN || "").replace(/\/+$/, "");
  const aud = String(env.ACCESS_AUD || "");
  // Access adds the header on the paths it protects; the CF_Authorization
  // cookie carries the same token everywhere else on the domain.
  const token = request.headers.get("Cf-Access-Jwt-Assertion") || cookie(request, "CF_Authorization");
  if (!team || !aud || !token) return null;
  try {
    const [head, body, signature] = token.split(".");
    if (!head || !body || !signature) return null;
    const header = decodeJson(head);
    const claims = decodeJson(body);
    if (header.alg !== "RS256") return null;
    const auds = Array.isArray(claims.aud) ? claims.aud : [claims.aud];
    if (!auds.includes(aud)) return null;
    if (claims.iss !== team) return null;
    const seconds = now / 1000;
    if (typeof claims.exp !== "number" || claims.exp < seconds) return null;
    if (typeof claims.nbf === "number" && claims.nbf > seconds + 60) return null;

    const jwk = (await keys(`${team}/cdn-cgi/access/certs`, fetcher, now)).find((k) => k.kid === header.kid);
    if (!jwk) return null;
    const key = await crypto.subtle.importKey(
      "jwk",
      jwk,
      { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" },
      false,
      ["verify"],
    );
    const ok = await crypto.subtle.verify(
      "RSASSA-PKCS1-v1_5",
      key,
      b64url(signature),
      new TextEncoder().encode(`${head}.${body}`),
    );
    return ok ? claims : null;
  } catch {
    return null;
  }
}

/// Tests only: forget cached certificates.
export function resetAccessCache() {
  cached = { url: "", at: 0, keys: null };
}

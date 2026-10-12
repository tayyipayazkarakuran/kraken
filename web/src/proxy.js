// URL and header rewriting for the /play/ proxy. Plain functions with no
// Worker APIs, so they run under `node --test`.
//
// A proxied page lives at /play/<host>/<path>: the host is part of the path,
// so relative URLs keep working and every upstream host the game touches
// gets its own cookie path.

export const PLAY = "/play/";

/// "a.com, *.b.com" → ["a.com", "*.b.com"]
export function hostList(text) {
  return String(text || "")
    .split(",")
    .map((h) => h.trim().toLowerCase())
    .filter(Boolean);
}

export function hostAllowed(host, patterns) {
  host = String(host || "").toLowerCase();
  if (!host || !/^[a-z0-9.-]+(:\d+)?$/.test(host)) return false;
  const bare = host.replace(/:\d+$/, "");
  return patterns.some((p) =>
    p.startsWith("*.") ? bare.endsWith(p.slice(1)) && bare.length > p.length - 1 : bare === p || host === p,
  );
}

/// "/play/www.a.com/x/y" → { host: "www.a.com", path: "/x/y" }
export function splitPlay(pathname) {
  if (!pathname.startsWith(PLAY)) return null;
  const rest = pathname.slice(PLAY.length);
  const slash = rest.indexOf("/");
  const host = slash < 0 ? rest : rest.slice(0, slash);
  if (!host) return null;
  return { host: host.toLowerCase(), path: slash < 0 ? "/" : rest.slice(slash) };
}

/// The upstream URL for a host and path; the game origin's own scheme is
/// kept (plain http only for local testing), everything else is https.
export function upstream(host, path, search, gameOrigin) {
  const game = new URL(gameOrigin);
  const scheme = game.host === host ? game.protocol : "https:";
  return `${scheme}//${host}${path}${search || ""}`;
}

/// An upstream URL as the proxy serves it, or `null` when the host is not
/// proxied. `base` resolves relative URLs (the upstream page's own URL).
export function toProxy(url, base, patterns) {
  let parsed;
  try {
    parsed = new URL(url, base);
  } catch {
    return null;
  }
  if (!/^(https?|wss?):$/.test(parsed.protocol)) return null;
  if (!hostAllowed(parsed.host, patterns)) return null;
  return `${PLAY}${parsed.host}${parsed.pathname}${parsed.search}${parsed.hash}`;
}

/// Rewrites a URL found in proxied HTML. Root-relative and absolute URLs to
/// proxied hosts move under /play/; relative ones already resolve there;
/// anything else is left alone.
export function rewriteAttribute(value, base, patterns) {
  const v = String(value).trim();
  if (!v || v.startsWith("#") || /^(data|blob|javascript|mailto|tel|about):/i.test(v)) return value;
  if (!v.startsWith("/") && !/^[a-z][a-z0-9+.-]*:/i.test(v)) return value;
  return toProxy(v, base, patterns) ?? value;
}

/// A redirect stays inside the proxy when it points at a proxied host.
export function rewriteLocation(value, base, patterns) {
  return toProxy(value, base, patterns) ?? value;
}

/// Upstream cookies become host-only cookies on our domain, scoped to the
/// upstream host's /play/ path.
export function rewriteSetCookie(value, host) {
  const [pair, ...attributes] = String(value).split(";");
  let path = "/";
  const kept = [];
  for (const attribute of attributes) {
    const [name, ...rest] = attribute.trim().split("=");
    const key = name.toLowerCase();
    if (key === "domain") continue;
    if (key === "path") {
      path = rest.join("=") || "/";
      continue;
    }
    kept.push(attribute.trim());
  }
  if (!path.startsWith("/")) path = `/${path}`;
  return [pair.trim(), `Path=${PLAY}${host}${path === "/" ? "/" : path}`, ...kept].join("; ");
}

/// Cookies our own site sets (Cloudflare Access) never travel upstream.
export function upstreamCookies(header) {
  return String(header || "")
    .split(";")
    .map((c) => c.trim())
    .filter((c) => c && !/^(CF_Authorization|CF_AppSession)=/i.test(c))
    .join("; ");
}

/// A root-relative request a proxied page made from script ("/api/x" from a
/// page at /play/www.a.com/…) arrives outside /play/; the Referer tells which
/// host it was meant for.
export function refererHost(referer, ourOrigin, patterns) {
  if (!referer) return null;
  let parsed;
  try {
    parsed = new URL(referer);
  } catch {
    return null;
  }
  if (parsed.origin !== ourOrigin) return null;
  const play = splitPlay(parsed.pathname);
  return play && hostAllowed(play.host, patterns) ? play.host : null;
}

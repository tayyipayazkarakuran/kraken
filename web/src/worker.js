// The Kraken site. Static files (the landing page, kraken-web under
// /__kraken/) are served before this Worker runs; it handles the rest:
//
//   /kraken.user.js   the userscript, pointed at this site's origin
//   /play/<host>/…    owner-only proxy (Cloudflare Access) that serves the
//                     game from this origin with kraken-boot.js injected
//
// The proxy never logs: no request or response bodies, headers or cookies
// are written anywhere.

import { verifyAccess } from "./access.js";
import {
  PLAY,
  hostAllowed,
  hostList,
  refererHost,
  rewriteAttribute,
  rewriteLocation,
  rewriteSetCookie,
  splitPlay,
  upstream,
  upstreamCookies,
} from "./proxy.js";

// Response headers that would stop the page from running under our origin
// or with our scripts.
const DROP_RESPONSE = [
  "content-security-policy",
  "content-security-policy-report-only",
  "x-frame-options",
  "strict-transport-security",
  "report-to",
  "nel",
];

// Request headers that belong to our site, not the game's.
const DROP_REQUEST = ["cf-access-jwt-assertion", "cf-access-authenticated-user-email", "cf-connecting-ip", "x-real-ip"];

// Attributes that carry URLs in the game's HTML.
const URL_ATTRIBUTES = ["href", "src", "action", "data-src", "poster"];

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/kraken.user.js") return userscript(env, url);
    if (url.pathname === "/play" || url.pathname.startsWith(PLAY)) return play(request, env, url);
    // A root-relative request from a proxied page ("/api/x") belongs under
    // /play/<host>/; only an authorised session gets redirected there.
    const patterns = hostList(env.PROXY_HOSTS);
    const host = refererHost(request.headers.get("Referer"), url.origin, patterns);
    if (host && (await verifyAccess(request, env))) {
      return Response.redirect(`${url.origin}${PLAY}${host}${url.pathname}${url.search}`, 307);
    }
    return new Response("Not found", { status: 404 });
  },
};

async function userscript(env, url) {
  const template = await env.ASSETS.fetch(new URL("/__kraken/userscript.js", url));
  if (!template.ok) return new Response("Run `cargo xtask web` first.", { status: 503 });
  const text = (await template.text())
    .replaceAll("__KRAKEN_ORIGIN__", url.origin)
    .replaceAll("__KRAKEN_HOST__", url.hostname);
  return new Response(text, {
    headers: {
      "content-type": "text/javascript; charset=utf-8",
      "cache-control": "no-cache",
    },
  });
}

async function play(request, env, url) {
  if (!(await verifyAccess(request, env))) {
    return new Response("Proxy mode is limited to the site owner (Cloudflare Access).", { status: 403 });
  }
  const patterns = hostList(env.PROXY_HOSTS);
  const target = splitPlay(url.pathname);
  if (!target) {
    // /play → the game page.
    const game = new URL(env.GAME_ORIGIN);
    return Response.redirect(`${url.origin}${PLAY}${game.host}${game.pathname}`, 302);
  }
  if (!hostAllowed(target.host, patterns)) return new Response("Host not allowed", { status: 403 });

  const upstreamUrl = upstream(target.host, target.path, url.search, env.GAME_ORIGIN);
  const upstreamOrigin = new URL(upstreamUrl).origin;
  const headers = new Headers(request.headers);
  for (const name of DROP_REQUEST) headers.delete(name);
  const cookies = upstreamCookies(headers.get("Cookie"));
  if (cookies) headers.set("Cookie", cookies);
  else headers.delete("Cookie");
  // The game sees its own origin, as if the page were loaded from it.
  if (headers.has("Origin")) headers.set("Origin", upstreamOrigin);
  const referer = headers.get("Referer");
  if (referer) {
    const from = URL.canParse(referer) ? splitPlay(new URL(referer).pathname) : null;
    if (from && hostAllowed(from.host, patterns)) {
      headers.set("Referer", upstream(from.host, from.path, "", env.GAME_ORIGIN));
    } else {
      headers.delete("Referer");
    }
  }

  // WebSockets pass straight through: the Worker hands the upgrade on.
  if (request.headers.get("Upgrade")?.toLowerCase() === "websocket") {
    return fetch(upstreamUrl, { headers });
  }

  const response = await fetch(upstreamUrl, {
    method: request.method,
    headers,
    body: ["GET", "HEAD"].includes(request.method) ? undefined : request.body,
    redirect: "manual",
  });

  const out = new Headers(response.headers);
  for (const name of DROP_RESPONSE) out.delete(name);
  const location = out.get("Location");
  if (location) out.set("Location", rewriteLocation(location, upstreamUrl, patterns));
  out.delete("Set-Cookie");
  for (const cookie of response.headers.getSetCookie?.() ?? []) {
    out.append("Set-Cookie", rewriteSetCookie(cookie, target.host));
  }

  const init = { status: response.status, statusText: response.statusText, headers: out };
  if (!(out.get("Content-Type") || "").includes("text/html")) return new Response(response.body, init);

  out.delete("Content-Length");
  const config = JSON.stringify({ hosts: patterns, origin: url.origin });
  const rewriter = new HTMLRewriter()
    .on("head", {
      element(head) {
        // First in <head>: before any of the game's own scripts run.
        head.prepend(
          `<script>window.__KRAKEN_PROXY__=${config.replace(/</g, "\\u003c")};</script>` +
            `<script src="/proxy-shim.js"></script>` +
            `<script src="/__kraken/kraken-boot.js"></script>`,
          { html: true },
        );
      },
    })
    .on("meta[http-equiv]", {
      element(meta) {
        if (/content-security-policy/i.test(meta.getAttribute("http-equiv") || "")) meta.remove();
      },
    });
  for (const name of URL_ATTRIBUTES) {
    rewriter.on(`[${name}]`, {
      element(element) {
        const value = element.getAttribute(name);
        const rewritten = rewriteAttribute(value, upstreamUrl, patterns);
        if (rewritten !== value) element.setAttribute(name, rewritten);
      },
    });
  }
  return rewriter.transform(new Response(response.body, init));
}

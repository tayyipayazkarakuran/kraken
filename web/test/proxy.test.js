import assert from "node:assert/strict";
import { test } from "node:test";

import {
  hostAllowed,
  hostList,
  refererHost,
  rewriteAttribute,
  rewriteLocation,
  rewriteSetCookie,
  splitPlay,
  toProxy,
  upstream,
  upstreamCookies,
} from "../src/proxy.js";

const hosts = hostList("seafight.com, *.seafight.com");
const page = "https://www.seafight.com/index.es?action=internalMapUnity";

test("host allowlist", () => {
  assert.ok(hostAllowed("seafight.com", hosts));
  assert.ok(hostAllowed("www.seafight.com", hosts));
  assert.ok(hostAllowed("alicdn-prod.seafight.com", hosts));
  assert.ok(!hostAllowed("evilseafight.com", hosts));
  assert.ok(!hostAllowed("seafight.com.evil.net", hosts));
  assert.ok(!hostAllowed("example.com", hosts));
  assert.ok(!hostAllowed("", hosts));
  assert.ok(!hostAllowed("a b.seafight.com", hosts));
  assert.ok(hostAllowed("127.0.0.1:8765", hostList("127.0.0.1:8765")));
});

test("play paths", () => {
  assert.deepEqual(splitPlay("/play/www.seafight.com/a/b"), { host: "www.seafight.com", path: "/a/b" });
  assert.deepEqual(splitPlay("/play/www.seafight.com"), { host: "www.seafight.com", path: "/" });
  assert.equal(splitPlay("/play/"), null);
  assert.equal(splitPlay("/other"), null);
});

test("upstream keeps the game origin's scheme only for the game host", () => {
  assert.equal(upstream("127.0.0.1:8765", "/x", "?a=1", "http://127.0.0.1:8765"), "http://127.0.0.1:8765/x?a=1");
  assert.equal(upstream("cdn.seafight.com", "/x", "", "http://127.0.0.1:8765"), "https://cdn.seafight.com/x");
});

test("URLs move under /play/ only for proxied hosts", () => {
  assert.equal(toProxy("https://cdn.seafight.com/a.wasm.gz", page, hosts), "/play/cdn.seafight.com/a.wasm.gz");
  assert.equal(toProxy("/js/app.js?v=2", page, hosts), "/play/www.seafight.com/js/app.js?v=2");
  assert.equal(toProxy("https://example.com/x", page, hosts), null);
  assert.equal(toProxy("javascript:alert(1)", page, hosts), null);
});

test("HTML attributes", () => {
  assert.equal(rewriteAttribute("/js/app.js", page, hosts), "/play/www.seafight.com/js/app.js");
  assert.equal(rewriteAttribute("js/app.js", page, hosts), "js/app.js");
  assert.equal(rewriteAttribute("#top", page, hosts), "#top");
  assert.equal(rewriteAttribute("data:image/png;base64,AA", page, hosts), "data:image/png;base64,AA");
  assert.equal(rewriteAttribute("https://cdn.example.net/x.js", page, hosts), "https://cdn.example.net/x.js");
  assert.equal(
    rewriteAttribute("https://alicdn-prod.seafight.com/u/Build.loader.js", page, hosts),
    "/play/alicdn-prod.seafight.com/u/Build.loader.js",
  );
});

test("redirects", () => {
  assert.equal(rewriteLocation("/index.es?action=internalStart", page, hosts), "/play/www.seafight.com/index.es?action=internalStart");
  assert.equal(rewriteLocation("https://accounts.example.com/login", page, hosts), "https://accounts.example.com/login");
});

test("cookies", () => {
  assert.equal(
    rewriteSetCookie("sid=abc; Domain=.seafight.com; Path=/; Secure; HttpOnly; SameSite=Lax", "www.seafight.com"),
    "sid=abc; Path=/play/www.seafight.com/; Secure; HttpOnly; SameSite=Lax",
  );
  assert.equal(rewriteSetCookie("a=1; path=/game", "x.seafight.com"), "a=1; Path=/play/x.seafight.com/game");
  assert.equal(upstreamCookies("CF_Authorization=jwt; sid=abc; CF_AppSession=s"), "sid=abc");
  assert.equal(upstreamCookies(""), "");
});

test("root-relative requests find their host through the referer", () => {
  const origin = "https://kraken.example";
  assert.equal(refererHost(`${origin}/play/www.seafight.com/index.es`, origin, hosts), "www.seafight.com");
  assert.equal(refererHost("https://other.example/play/www.seafight.com/", origin, hosts), null);
  assert.equal(refererHost(`${origin}/play/example.com/`, origin, hosts), null);
  assert.equal(refererHost("not a url", origin, hosts), null);
});

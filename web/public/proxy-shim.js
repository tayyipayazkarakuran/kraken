// Proxy mode only (/play/): keeps the game's own network calls inside the
// proxy. Absolute URLs to proxied hosts ("https://x.seafight.com/a",
// "wss://…") and root-relative ones ("/a") are mapped to /play/<host>/…;
// everything else is left alone. The Worker injects this ahead of every page
// script, with `window.__KRAKEN_PROXY__ = { hosts, origin }`.
(() => {
  "use strict";
  const config = window.__KRAKEN_PROXY__;
  if (!config || window.__krakenShim) return;
  window.__krakenShim = true;

  const PLAY = "/play/";
  const patterns = config.hosts || [];

  function allowed(host) {
    const bare = host.toLowerCase().replace(/:\d+$/, "");
    return patterns.some((p) =>
      p.startsWith("*.") ? bare.endsWith(p.slice(1)) && bare.length > p.length - 1 : bare === p || host === p,
    );
  }

  // The upstream URL of this page, from /play/<host>/<path>.
  function pageUpstream() {
    const rest = location.pathname.slice(PLAY.length);
    const slash = rest.indexOf("/");
    const host = slash < 0 ? rest : rest.slice(0, slash);
    const path = slash < 0 ? "/" : rest.slice(slash);
    return `https://${host}${path}${location.search}`;
  }

  function map(url, websocket) {
    if (typeof url !== "string" && !(url instanceof URL)) return url;
    const text = String(url);
    if (text.startsWith(PLAY) || text.startsWith(`${location.origin}${PLAY}`)) return url;
    let parsed;
    try {
      parsed = new URL(text, pageUpstream());
    } catch {
      return url;
    }
    if (!allowed(parsed.host)) return url;
    const path = `${PLAY}${parsed.host}${parsed.pathname}${parsed.search}`;
    if (websocket) return `${location.protocol === "https:" ? "wss:" : "ws:"}//${location.host}${path}`;
    return `${location.origin}${path}`;
  }

  // Relative URLs that do not start with "/" already resolve under /play/.
  const needsMap = (url) => {
    const text = String(url);
    return text.startsWith("/") || /^[a-z][a-z0-9+.-]*:\/\//i.test(text);
  };

  const pageFetch = window.fetch;
  window.fetch = function (input, init) {
    if (typeof input === "string" || input instanceof URL) {
      if (needsMap(input)) input = map(input, false);
    } else if (input && input.url && needsMap(input.url)) {
      const mapped = map(input.url, false);
      if (mapped !== input.url) input = new Request(mapped, input);
    }
    return pageFetch.call(this, input, init);
  };

  const open = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (method, url, ...rest) {
    return open.call(this, method, needsMap(url) ? map(url, false) : url, ...rest);
  };

  const PageSocket = window.WebSocket;
  window.WebSocket = new Proxy(PageSocket, {
    construct(target, [url, ...rest]) {
      return new target(map(url, true), ...rest);
    },
  });
})();

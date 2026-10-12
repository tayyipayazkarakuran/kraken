// Kraken web bootstrap. Runs before any page script in every frame, and stays
// idle in frames where Unity never loads. It gets there one of three ways:
// the Kraken userscript (`kraken.user.js`, any browser with a userscript
// manager), the site's proxy mode (`/play/`, injected by the Worker), or the
// Android shell (`addDocumentStartJavaScript`).
//
// It catches the Unity WebAssembly instance as it is created, so Kraken can
// reach the game's linear memory and function table, then loads kraken-web
// next to it and shows the "K" panel. `__kraken.probe()` reports what the
// game module exposes (are the il2cpp_* functions exported, is there a name
// section?).
(() => {
  "use strict";
  if (window.__kraken) return;
  // Visible from the userscript's own world: the bootstrap made it in.
  if (document.documentElement) document.documentElement.dataset.kraken = "boot";

  const TAG = "[kraken]";
  const TICK_MS = 100;
  const log = (...args) => console.log(TAG, ...args);

  const state = {
    // Every module instantiated in this frame: { module, instance, imports, bytes }.
    modules: [],
    // wasm downloads seen through `fetch`: { url, size }.
    downloads: [],
    kraken: null, // the kraken-web `Kraken`
    version: null,
    error: null,
    timer: 0,
  };

  // Where kraken-web comes from without the userscript: the page's own origin
  // (`/__kraken/` is answered by the Android shell and the proxy Worker).
  // Desktop testing can point this elsewhere before the game loads.
  const base = () => window.__KRAKEN_BASE__ || `${location.origin}/__kraken/`;

  function record(result, imports, bytes) {
    // `instantiate(bytes)` resolves to { module, instance };
    // `instantiate(module)` resolves to the instance alone.
    const entry =
      result instanceof WebAssembly.Instance
        ? { module: null, instance: result, imports, bytes }
        : { module: result.module, instance: result.instance, imports, bytes };
    state.modules.push(entry);
    const exports = Object.keys(entry.instance.exports);
    log(`wasm instantiated: ${exports.length} exports`);
    if (game()) start();
    return result;
  }

  const instantiate = WebAssembly.instantiate;
  WebAssembly.instantiate = function (source, imports) {
    const bytes = source instanceof WebAssembly.Module ? null : source;
    return instantiate.call(this, source, imports).then((r) => record(r, imports, bytes));
  };

  const streaming = WebAssembly.instantiateStreaming;
  if (streaming) {
    WebAssembly.instantiateStreaming = function (source, imports) {
      return streaming.call(this, source, imports).then((r) => record(r, imports, null));
    };
  }

  // Unity fetches its code as `*.wasm`, `*.wasm.gz`, `*.wasm.br` or
  // `*.unityweb`. Note which file and how big; the response itself passes
  // through untouched.
  const WASM_URL = /\.(wasm|unityweb)(\.(gz|br))?($|[?#])/i;
  const pageFetch = window.fetch;
  if (pageFetch) {
    window.fetch = function (input, init) {
      const url = typeof input === "string" ? input : input && input.url;
      const response = pageFetch.call(this, input, init);
      if (url && WASM_URL.test(url) && !url.includes("kraken_web")) {
        response
          .then((r) => r.clone().arrayBuffer())
          .then((buffer) => {
            state.downloads.push({ url, size: buffer.byteLength });
            panel.update();
          })
          .catch(() => {});
      }
      return response;
    };
  }

  /// The Unity module: the instance with a memory and a function table
  /// (exported, or imported from `env` in older Emscripten builds), and the
  /// largest one if several have both.
  function game() {
    let best = null;
    for (const entry of state.modules) {
      const values = [
        ...Object.values(entry.instance.exports),
        ...Object.values((entry.imports && entry.imports.env) || {}),
      ];
      const memory = values.find((v) => v instanceof WebAssembly.Memory);
      const table = values.find((v) => v instanceof WebAssembly.Table);
      if (!memory || !table) continue;
      if (!best || table.length > best.table.length) {
        best = { ...entry, memory, table };
      }
    }
    return best;
  }

  function probe() {
    const g = game();
    const report = {
      page: location.href,
      kraken: state.version,
      error: state.error,
      downloads: state.downloads,
      wasm: null,
    };
    if (g) {
      const names = Object.keys(g.instance.exports);
      report.wasm = {
        exports: names.length,
        il2cpp: names.filter((n) => n.startsWith("il2cpp_")),
        sample: names.slice(0, 40),
        memoryMiB: g.memory.buffer.byteLength / (1 << 20),
        table: g.table.length,
        nameSection: g.module ? WebAssembly.Module.customSections(g.module, "name").length > 0 : "unknown",
        // Unity's own loader result, when the page keeps it in a global.
        unity: !!(window.unityInstance || window.gameInstance),
      };
    }
    log("probe", report);
    return report;
  }

  function addScript(attributes) {
    return new Promise((resolve, reject) => {
      const script = document.createElement("script");
      Object.assign(script, attributes);
      script.onload = resolve;
      script.onerror = () => reject(new Error(`load ${attributes.src}`));
      (document.head || document.documentElement).appendChild(script);
      // Inline scripts run synchronously and fire no load event.
      if (!attributes.src) resolve();
    });
  }

  /// The wasm-bindgen glue and module. The userscript cannot rely on the
  /// page's CSP letting our origin in, so it fetches both itself and posts
  /// them here; everyone else loads them from `base()`.
  function assets() {
    if (!window.__KRAKEN_ASSETS_PENDING__) {
      return Promise.resolve({ glue: null, wasm: `${base()}kraken_web_bg.wasm` });
    }
    return new Promise((resolve) => {
      window.addEventListener("message", function listen(event) {
        const data = event.data && event.data.__krakenAssets;
        // Only the userscript, running against this very window, may hand
        // over code.
        if (!data || event.source !== window) return;
        window.removeEventListener("message", listen);
        resolve(data);
      });
      window.postMessage({ __krakenNeedAssets: true }, "*");
    });
  }

  let starting = false;
  async function start() {
    if (starting) return;
    starting = true;
    panel.update();
    try {
      const { glue, wasm } = await assets();
      // `wasm-bindgen --target no-modules` declares a global `let
      // wasm_bindgen`: a script-scope binding, not a `window` property.
      if (typeof wasm_bindgen === "undefined") {
        await addScript(glue ? { textContent: glue } : { src: `${base()}kraken_web.js` });
      }
      const bindings = wasm_bindgen; // eslint-disable-line no-undef
      const own = await bindings({ module_or_path: wasm });
      // Our own module went through the hook above too; it is not the game.
      state.modules = state.modules.filter((entry) => entry.instance.exports !== own);
      const { Kraken, version } = bindings;
      state.kraken = new Kraken(BigInt(Date.now()));
      state.version = version();
      state.error = null;
      log(`kraken-web ${state.version} loaded`);
    } catch (error) {
      starting = false;
      state.error = String(error);
      console.error(TAG, error);
    }
    panel.update();
  }

  function run(command) {
    const kraken = state.kraken;
    if (!kraken) return log("kraken-web not loaded");
    report(kraken.control(BigInt(Date.now()), command));
    clearInterval(state.timer);
    if (kraken.running) {
      // Later this moves into Unity's own frame callback, where calls into
      // the game are safe.
      state.timer = setInterval(() => report(kraken.tick(BigInt(Date.now()))), TICK_MS);
    }
    panel.update();
  }

  function report(actions) {
    if (actions.length) log("actions", actions);
  }

  // The "K" button and panel, in a shadow root so the game's CSS cannot
  // reach it. Shown in the frame that runs the game, unless a native shell
  // (Android) brings its own menu.
  const panel = (() => {
    let host = null;
    let parts = null;

    const STYLE = `
      :host { all: initial; }
      * { box-sizing: border-box; font: 14px/1.4 system-ui, sans-serif; }
      .k { position: fixed; right: 12px; bottom: 12px; z-index: 2147483647;
           width: 44px; height: 44px; border-radius: 50%; border: 0;
           background: rgba(16, 58, 84, .85); color: #fff; font-weight: 700; font-size: 18px; }
      .p { position: fixed; right: 12px; bottom: 64px; z-index: 2147483647;
           width: min(320px, calc(100vw - 24px)); max-height: calc(100vh - 88px); overflow: auto;
           padding: 12px; border-radius: 12px; background: rgba(12, 22, 30, .94); color: #e6eef3;
           box-shadow: 0 6px 24px rgba(0, 0, 0, .4); }
      .p[hidden] { display: none; }
      .s { margin: 0 0 10px; color: #9fb4c2; }
      .s b { color: #e6eef3; font-weight: 600; }
      .row { display: flex; gap: 6px; flex-wrap: wrap; margin-bottom: 8px; }
      button.a { flex: 1 1 auto; min-height: 40px; padding: 0 10px; border-radius: 8px; border: 0;
                 background: #1f5f86; color: #fff; }
      button.a:disabled { opacity: .45; }
      pre { margin: 8px 0 0; padding: 8px; max-height: 40vh; overflow: auto; border-radius: 8px;
            background: #071017; color: #cfe3ee; font: 11px/1.35 ui-monospace, monospace; white-space: pre-wrap;
            word-break: break-all; }
      pre:empty { display: none; }
    `;

    function create() {
      if (host || window.__KRAKEN_NATIVE__ || !document.body) return;
      host = document.createElement("kraken-panel");
      const root = host.attachShadow({ mode: "closed" });
      root.innerHTML = `
        <style>${STYLE}</style>
        <button class="k" aria-label="Kraken">K</button>
        <div class="p" hidden>
          <p class="s"></p>
          <div class="row">
            <button class="a" data-c="start">Başlat</button>
            <button class="a" data-c="pause">Duraklat</button>
            <button class="a" data-c="stop">Durdur</button>
          </div>
          <div class="row">
            <button class="a" data-c="probe">İncele</button>
            <button class="a" data-c="copy">Kopyala</button>
          </div>
          <pre></pre>
        </div>`;
      parts = {
        sheet: root.querySelector(".p"),
        status: root.querySelector(".s"),
        out: root.querySelector("pre"),
        buttons: [...root.querySelectorAll("button.a")],
      };
      root.querySelector(".k").addEventListener("click", () => {
        parts.sheet.hidden = !parts.sheet.hidden;
        update();
      });
      for (const button of parts.buttons) {
        button.addEventListener("click", () => {
          const c = button.dataset.c;
          if (c === "probe") parts.out.textContent = JSON.stringify(probe(), null, 2);
          else if (c === "copy") copy(parts.out.textContent || JSON.stringify(probe(), null, 2));
          else run(c);
        });
      }
      document.body.appendChild(host);
      update();
    }

    function copy(text) {
      const done = () => (parts.status.textContent = "Panoya kopyalandı.");
      if (navigator.clipboard) navigator.clipboard.writeText(text).then(done, () => {});
    }

    function update() {
      if (!game()) return;
      if (!host) {
        if (document.body) create();
        else document.addEventListener("DOMContentLoaded", create, { once: true });
        return;
      }
      const kraken = state.kraken;
      const status = state.error
        ? `Hata: <b>${escape(state.error)}</b>`
        : kraken
          ? `Kraken <b>${escape(state.version)}</b> · ${kraken.running ? "<b>çalışıyor</b>" : "duruyor"}`
          : "Kraken yükleniyor…";
      parts.status.innerHTML = `${status}<br>Oyun modülü yakalandı.`;
      for (const button of parts.buttons) {
        if (["start", "pause", "stop"].includes(button.dataset.c)) button.disabled = !kraken;
      }
    }

    const escape = (text) =>
      String(text).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

    return { update };
  })();

  window.__kraken = {
    state,
    game,
    probe,
    start: () => run("start"),
    pause: () => run("pause"),
    stop: () => run("stop"),
  };

  // Commands from a native shell or another frame: `{ __kraken: "start" }`.
  // Each frame passes them down to its children, since the game may sit in
  // an iframe.
  window.addEventListener("message", (event) => {
    const command = event.data && event.data.__kraken;
    if (typeof command !== "string") return;
    for (let i = 0; i < window.frames.length; i++) {
      window.frames[i].postMessage(event.data, "*");
    }
    if (!game()) return;
    if (command === "probe") probe();
    else run(command);
  });
})();

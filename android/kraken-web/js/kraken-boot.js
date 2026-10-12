// Kraken web bootstrap. Runs before any page script (Android:
// `addDocumentStartJavaScript`; desktop: paste into DevTools before the game
// loads, or a userscript at document-start) in every frame, and stays idle in
// frames where Unity never loads.
//
// It catches the Unity WebAssembly instance as it is created, so Kraken can
// reach the game's linear memory and function table, then loads kraken-web
// next to it. `window.__kraken.probe()` prints what the game module exposes
// (Phase 0 spike: are the il2cpp_* functions exported, is there a name
// section?).
(() => {
  "use strict";
  if (window.__kraken) return;

  const TAG = "[kraken]";
  const TICK_MS = 100;
  const log = (...args) => console.log(TAG, ...args);

  const state = {
    // Every module instantiated in this frame: { module, instance, imports, bytes }.
    modules: [],
    kraken: null, // the kraken-web `Kraken`
    timer: 0,
  };

  // kraken-web is served from the game's own origin (the Android shell
  // answers `/__kraken/*` from its assets), so the page's CSP allows it.
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
    if (!g) {
      log("no Unity module yet");
      return null;
    }
    const names = Object.keys(g.instance.exports);
    const report = {
      exports: names.length,
      il2cpp: names.filter((n) => n.startsWith("il2cpp_")),
      memoryMiB: g.memory.buffer.byteLength / (1 << 20),
      table: g.table.length,
      nameSection: g.module ? WebAssembly.Module.customSections(g.module, "name").length > 0 : "unknown",
      // Unity's own loader result, when the page keeps it in a global.
      unity: !!(window.unityInstance || window.gameInstance),
    };
    console.table(report);
    return report;
  }

  function loadScript(url) {
    return new Promise((resolve, reject) => {
      const script = document.createElement("script");
      script.src = url;
      script.onload = resolve;
      script.onerror = () => reject(new Error(`load ${url}`));
      (document.head || document.documentElement).appendChild(script);
    });
  }

  let starting = false;
  async function start() {
    if (starting) return;
    starting = true;
    try {
      // `wasm-bindgen --target no-modules` declares a global `let
      // wasm_bindgen`: a script-scope binding, not a `window` property.
      if (typeof wasm_bindgen === "undefined") await loadScript(`${base()}kraken_web.js`);
      const bindings = wasm_bindgen; // eslint-disable-line no-undef
      const own = await bindings({ module_or_path: `${base()}kraken_web_bg.wasm` });
      // Our own module went through the hook above too; it is not the game.
      state.modules = state.modules.filter((entry) => entry.instance.exports !== own);
      const { Kraken, version } = bindings;
      state.kraken = new Kraken(BigInt(Date.now()));
      log(`kraken-web ${version()} loaded`);
      probe();
    } catch (error) {
      starting = false;
      console.error(TAG, error);
    }
  }

  function run(command) {
    const kraken = state.kraken;
    if (!kraken) return log("kraken-web not loaded");
    report(kraken.control(BigInt(Date.now()), command));
    clearInterval(state.timer);
    if (kraken.running) {
      // Phase 2 moves this into Unity's own frame callback, where calls into
      // the game are safe.
      state.timer = setInterval(() => report(kraken.tick(BigInt(Date.now()))), TICK_MS);
    }
  }

  function report(actions) {
    if (actions.length) log("actions", actions);
  }

  window.__kraken = {
    state,
    game,
    probe,
    start: () => run("start"),
    pause: () => run("pause"),
    stop: () => run("stop"),
  };

  // The Android shell can only reach the top frame, and the game may sit in
  // an iframe: commands arrive as `{ __kraken: "start" }` messages, and each
  // frame passes them down to its children.
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

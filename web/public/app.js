// Shows the install steps for the visitor's platform; the tabs switch them.
(() => {
  const ua = navigator.userAgent;
  const ios = /iPhone|iPad|iPod/.test(ua) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
  const detected = ios ? "ios" : /Android/.test(ua) ? "android" : "pc";

  const tabs = [...document.querySelectorAll("[data-platform]")];
  const steps = [...document.querySelectorAll(".steps")];

  function show(platform) {
    for (const tab of tabs) tab.setAttribute("aria-selected", String(tab.dataset.platform === platform));
    for (const list of steps) list.hidden = list.dataset.for !== platform;
  }

  for (const tab of tabs) tab.addEventListener("click", () => show(tab.dataset.platform));
  show(detected);
})();

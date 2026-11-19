// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/shared/browser/guest-lifecycle/browser-viewport-presets.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

/** The sizes a page can be shown at, matching Chrome DevTools' own; a mobile one turns touch and small-screen CSS on. */
export const PRESETS = [
  { id: "mobile-s", label: "Mobile S — 320 × 568", width: 320, height: 568, scale: 2, mobile: true },
  { id: "mobile-m", label: "Mobile M — 375 × 667", width: 375, height: 667, scale: 2, mobile: true },
  { id: "mobile-l", label: "Mobile L — 425 × 812", width: 425, height: 812, scale: 2, mobile: true },
  { id: "tablet", label: "Tablet — 768 × 1024", width: 768, height: 1024, scale: 2, mobile: true },
  { id: "laptop", label: "Laptop — 1024 × 768", width: 1024, height: 768, scale: 1, mobile: false },
  { id: "laptop-l", label: "Laptop L — 1440 × 900", width: 1440, height: 900, scale: 1, mobile: false },
  { id: "desktop", label: "Desktop — 1920 × 1080", width: 1920, height: 1080, scale: 1, mobile: false },
];

export const presetOf = (id) => PRESETS.find((p) => p.id === id) || null;

/** A viewport as asked for (a preset's name, or a size), checked; null for the page's own size. */
export function viewportOf(asked) {
  if (!asked) return null;
  if (typeof asked === "string") {
    const preset = presetOf(asked);
    return preset ? { preset: preset.id, width: preset.width, height: preset.height, scale: preset.scale, mobile: preset.mobile } : null;
  }
  if (asked.preset) return viewportOf(asked.preset);
  const width = Math.round(Number(asked.width));
  const height = Math.round(Number(asked.height));
  if (!(width >= 200 && width <= 7680 && height >= 200 && height <= 7680)) return null;
  const scale = Number(asked.scale) >= 1 && Number(asked.scale) <= 4 ? Number(asked.scale) : 1;
  return { preset: null, width, height, scale, mobile: asked.mobile === true };
}

/** The scale at which a viewport fits a box without scrolling, at most its own size. */
export function fitScale(viewport, box) {
  if (!viewport || box.width <= 0 || box.height <= 0) return 1;
  return Math.min(1, box.width / viewport.width, box.height / viewport.height);
}

/** The Chrome DevTools Protocol call that shows a page at a viewport, and the one that gives it its own size back. */
export function emulation(viewport) {
  if (!viewport) return [["Emulation.clearDeviceMetricsOverride", {}], ["Emulation.setTouchEmulationEnabled", { enabled: false }]];
  return [
    ["Emulation.setDeviceMetricsOverride", { width: viewport.width, height: viewport.height, deviceScaleFactor: viewport.scale, mobile: viewport.mobile }],
    ["Emulation.setTouchEmulationEnabled", { enabled: viewport.mobile }],
  ];
}

// Tiny DOM helpers — no framework.

type Attrs = Record<string, string | number | boolean | undefined>;

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  ...children: (Node | string | null | undefined)[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === undefined || v === false) continue;
    if (k === 'class') el.className = String(v);
    else if (k === 'text') el.textContent = String(v);
    else el.setAttribute(k, v === true ? '' : String(v));
  }
  for (const c of children) {
    if (c !== null && c !== undefined) el.append(c);
  }
  return el;
}

/** Page visibility + our own tray hide/show events (WebView may not report hidden). */
let windowShown = true;
const visibilityListeners = new Set<(visible: boolean) => void>();

export function isVisible(): boolean {
  return windowShown && !document.hidden;
}

export function setWindowShown(shown: boolean): void {
  windowShown = shown;
  notify();
}

export function onVisibilityChange(cb: (visible: boolean) => void): void {
  visibilityListeners.add(cb);
}

function notify(): void {
  const v = isVisible();
  visibilityListeners.forEach((cb) => cb(v));
}

document.addEventListener('visibilitychange', notify);

// ---- toasts (flat, bordered, no shadow)

export function toast(message: string, kind: 'error' | 'info' = 'info', ms = 5000): void {
  const host = document.getElementById('toasts');
  if (!host) return;
  const el = h('div', { class: `toast toast-${kind}`, role: kind === 'error' ? 'alert' : 'status', text: message });
  host.append(el);
  while (host.children.length > 4) host.firstElementChild?.remove();
  window.setTimeout(() => el.remove(), ms);
}

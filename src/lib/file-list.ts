// Yerel ve uzak dosya panellerinin ortak davranışı: sıralama ve dosya yöneticisi tarzı seçim.
import type { Entry } from "./api";

export type SortKey = "name" | "size" | "mtime";
export interface Sort {
  key: SortKey;
  desc: boolean;
}

// "Proje 2" < "Proje 10"; büyük/küçük harf ve Türkçe harfler doğal sırada.
const collator = new Intl.Collator("tr", { numeric: true, sensitivity: "base" });

/** Klasörler her zaman üstte; eşitlikte ada göre. */
export function sortEntries(list: Entry[], s: Sort): Entry[] {
  const dir = s.desc ? -1 : 1;
  return [...list].sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    if (s.key === "name") return collator.compare(a.name, b.name) * dir;
    const c = s.key === "size" ? a.size - b.size : (a.mtime ?? 0) - (b.mtime ?? 0);
    // Eşitlikte ada göre (her zaman A→Z).
    return c !== 0 ? c * dir : collator.compare(a.name, b.name);
  });
}

/** Aynı başlığa tıklayınca yön değişir; yeni başlıkta ad A→Z, boyut ve tarih büyükten/yeniden başlar. */
export function toggleSort(cur: Sort, key: SortKey): Sort {
  if (cur.key === key) return { key, desc: !cur.desc };
  return { key, desc: key !== "name" };
}

export interface SelState {
  selected: Set<string>;
  /** Shift ile aralık seçiminin başlangıcı. */
  anchor: string | null;
  /** Klavye imleci (odaktaki satır). */
  cursor: string | null;
}

const isMac = typeof navigator !== "undefined" && navigator.platform.toLowerCase().includes("mac");
const toggleKey = (ev: MouseEvent | KeyboardEvent) => (isMac ? ev.metaKey : ev.ctrlKey);

function range(order: string[], from: string | null, to: string): string[] {
  const i = from ? order.indexOf(from) : -1;
  const j = order.indexOf(to);
  if (i < 0 || j < 0) return [to];
  const [a, b] = i < j ? [i, j] : [j, i];
  return order.slice(a, b + 1);
}

/** Tıklama: düz tık tek seçer, Ctrl/Cmd ekler-çıkarır, Shift aralık seçer (Ctrl/Cmd+Shift ekler). */
export function clickSel(st: SelState, ev: MouseEvent, path: string, order: string[]): SelState {
  if (ev.shiftKey) {
    const span = range(order, st.anchor ?? st.cursor, path);
    const selected = toggleKey(ev) ? new Set([...st.selected, ...span]) : new Set(span);
    return { selected, anchor: st.anchor ?? path, cursor: path };
  }
  if (toggleKey(ev)) {
    const selected = new Set(st.selected);
    if (selected.has(path)) selected.delete(path);
    else selected.add(path);
    return { selected, anchor: path, cursor: path };
  }
  return { selected: new Set([path]), anchor: path, cursor: path };
}

/**
 * Seçimle ilgili tuşlar: oklar, Home/End, PageUp/PageDown (Shift ile aralık), Ctrl/Cmd+A, Esc.
 * İşlenmeyen tuşta `null` döner.
 */
export function keySel(st: SelState, ev: KeyboardEvent, order: string[], page = 10): SelState | null {
  if (toggleKey(ev) && !ev.shiftKey && !ev.altKey && ev.key.toLowerCase() === "a") {
    return { selected: new Set(order), anchor: order[0] ?? null, cursor: st.cursor ?? order[0] ?? null };
  }
  if (ev.key === "Escape") {
    if (!st.selected.size) return null;
    return { selected: new Set(), anchor: null, cursor: st.cursor };
  }
  if (!order.length) return null;
  const cur = st.cursor ? order.indexOf(st.cursor) : -1;
  let next: number;
  switch (ev.key) {
    case "ArrowDown":
      next = cur < 0 ? 0 : Math.min(order.length - 1, cur + 1);
      break;
    case "ArrowUp":
      next = cur < 0 ? order.length - 1 : Math.max(0, cur - 1);
      break;
    case "Home":
      next = 0;
      break;
    case "End":
      next = order.length - 1;
      break;
    case "PageDown":
      next = Math.min(order.length - 1, Math.max(cur, 0) + page);
      break;
    case "PageUp":
      next = Math.max(0, cur - page);
      break;
    default:
      return null;
  }
  const path = order[next];
  if (ev.shiftKey) {
    const anchor = st.anchor ?? st.cursor ?? path;
    return { selected: new Set(range(order, anchor, path)), anchor, cursor: path };
  }
  return { selected: new Set([path]), anchor: path, cursor: path };
}

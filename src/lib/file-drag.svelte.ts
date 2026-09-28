// Yerel ve uzak dosya panelleri arasında sürükle-bırak.
// Tarayıcının HTML5 sürükle-bırakı Windows'ta Tauri'nin dosya bırakma özelliğiyle çakıştığı için
// işaretçi olaylarıyla yapılır.
import type { Entry } from "./api";

export type Side = "local" | "remote";

type Drop = (items: Entry[], from: Side) => void;

class FileDrag {
  drag = $state<{ from: Side; items: Entry[]; x: number; y: number; over: Side | null } | null>(null);
  private zones = new Map<Side, Drop>();
  private press: { from: Side; items: () => Entry[]; x: number; y: number } | null = null;

  /** Bırakma bölgesini kaydeder; kaldırmak için dönen fonksiyonu çağırın. */
  zone(side: Side, onDrop: Drop) {
    this.zones.set(side, onDrop);
    return () => {
      if (this.zones.get(side) === onDrop) this.zones.delete(side);
    };
  }

  /** Sürüklemeden aktarım (panel düğmeleri için). */
  deliver(to: Side, items: Entry[], from: Side) {
    if (items.length) this.zones.get(to)?.(items, from);
  }

  has(side: Side) {
    return this.zones.has(side);
  }

  /** Satırda sol tuşa basıldığında çağrılır; 6 pikselden fazla hareket sürüklemeyi başlatır. */
  start(ev: PointerEvent, from: Side, items: () => Entry[]) {
    if (ev.button !== 0 || ev.metaKey || ev.ctrlKey || ev.shiftKey) return;
    this.press = { from, items, x: ev.clientX, y: ev.clientY };
    window.addEventListener("pointermove", this.move);
    window.addEventListener("pointerup", this.up, { once: true });
  }

  private move = (ev: PointerEvent) => {
    if (!this.press) return;
    if (!this.drag) {
      if (Math.hypot(ev.clientX - this.press.x, ev.clientY - this.press.y) < 6) return;
      const items = this.press.items();
      if (!items.length) return;
      this.drag = { from: this.press.from, items, x: 0, y: 0, over: null };
    }
    const el = document.elementFromPoint(ev.clientX, ev.clientY)?.closest<HTMLElement>("[data-file-drop]");
    const over = (el?.dataset.fileDrop as Side | undefined) ?? null;
    this.drag.x = ev.clientX;
    this.drag.y = ev.clientY;
    this.drag.over = over && over !== this.drag.from ? over : null;
  };

  private up = () => {
    window.removeEventListener("pointermove", this.move);
    const d = this.drag;
    this.press = null;
    this.drag = null;
    if (d?.over) this.zones.get(d.over)?.(d.items, d.from);
  };
}

export const fileDrag = new FileDrag();

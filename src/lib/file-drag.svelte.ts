// Yerel ve uzak dosya panelleri arasında sürükle-bırak.
// Tarayıcının HTML5 sürükle-bırakı Windows'ta Tauri'nin dosya bırakma özelliğiyle çakıştığı için
// işaretçi olaylarıyla yapılır.
import type { Entry } from "./api";

export type Side = "local" | "remote";

type Drop = (items: Entry[], from: Side) => void;

// Her SFTP sekmesinin yerel ve uzak paneli bir "grup"tur (bağlantı kimliği); aktarım yalnızca
// aynı grubun panelleri arasında olur, yoksa arka plandaki başka sekmenin sunucusuna gidebilir.
const key = (side: Side, group: string) => `${side}|${group}`;

class FileDrag {
  drag = $state<{ from: Side; group: string; items: Entry[]; x: number; y: number; over: Side | null } | null>(null);
  private zones = new Map<string, Drop>();
  private press: { from: Side; group: string; items: () => Entry[]; x: number; y: number } | null = null;

  /** Bırakma bölgesini kaydeder; kaldırmak için dönen fonksiyonu çağırın. */
  zone(side: Side, group: string, onDrop: Drop) {
    const k = key(side, group);
    this.zones.set(k, onDrop);
    return () => {
      if (this.zones.get(k) === onDrop) this.zones.delete(k);
    };
  }

  /** Sürüklemeden aktarım (panel düğmeleri için). */
  deliver(to: Side, group: string, items: Entry[], from: Side) {
    if (items.length) this.zones.get(key(to, group))?.(items, from);
  }

  /** Bu gruptaki `side` panelinin üzerine mi sürükleniyor? */
  isOver(side: Side, group: string) {
    return this.drag?.over === side && this.drag.group === group;
  }

  /** Satırda sol tuşa basıldığında çağrılır; 6 pikselden fazla hareket sürüklemeyi başlatır. */
  start(ev: PointerEvent, from: Side, group: string, items: () => Entry[]) {
    if (ev.button !== 0 || ev.metaKey || ev.ctrlKey || ev.shiftKey) return;
    this.press = { from, group, items, x: ev.clientX, y: ev.clientY };
    window.addEventListener("pointermove", this.move);
    window.addEventListener("pointerup", this.up, { once: true });
  }

  private move = (ev: PointerEvent) => {
    if (!this.press) return;
    if (!this.drag) {
      if (Math.hypot(ev.clientX - this.press.x, ev.clientY - this.press.y) < 6) return;
      const items = this.press.items();
      if (!items.length) return;
      this.drag = { from: this.press.from, group: this.press.group, items, x: 0, y: 0, over: null };
    }
    const el = document.elementFromPoint(ev.clientX, ev.clientY)?.closest<HTMLElement>("[data-file-drop]");
    const over = el?.dataset.fileGroup === this.drag.group ? ((el.dataset.fileDrop as Side | undefined) ?? null) : null;
    this.drag.x = ev.clientX;
    this.drag.y = ev.clientY;
    this.drag.over = over && over !== this.drag.from ? over : null;
  };

  private up = () => {
    window.removeEventListener("pointermove", this.move);
    const d = this.drag;
    this.press = null;
    this.drag = null;
    if (d?.over) this.zones.get(key(d.over, d.group))?.(d.items, d.from);
  };
}

export const fileDrag = new FileDrag();

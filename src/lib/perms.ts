// İzin (chmod) penceresinin saf mantığı: sekizli yazı ↔ bitler.

export type ChmodScope = "item" | "all" | "files" | "dirs";

/** Sahip/grup/diğerleri × okuma/yazma/çalıştırma bitleri, ekrandaki sırayla. */
export const WHO = [
  { label: "Sahip", shift: 6 },
  { label: "Grup", shift: 3 },
  { label: "Diğerleri", shift: 0 },
] as const;
export const WHAT = [
  { label: "Okuma", bit: 4 },
  { label: "Yazma", bit: 2 },
  { label: "Çalıştırma", bit: 1 },
] as const;
export const SPECIAL = [
  { label: "setuid", bit: 0o4000 },
  { label: "setgid", bit: 0o2000 },
  { label: "sticky", bit: 0o1000 },
] as const;

/** "755", "0755", "4755" gibi 3–4 sekizli rakamı izin bitlerine çevirir; geçersizse null. */
export function parseOctal(text: string): number | null {
  const t = text.trim();
  if (!/^[0-7]{3,4}$/.test(t)) return null;
  return parseInt(t, 8);
}

/** Özel bit yoksa 3, varsa 4 rakam: 0o755 → "755", 0o4755 → "4755". */
export function toOctal(mode: number): string {
  const m = mode & 0o7777;
  return m.toString(8).padStart(m > 0o777 ? 4 : 3, "0");
}

export function toggle(mode: number, bit: number, on: boolean): number {
  return on ? mode | bit : mode & ~bit;
}

/** Seçilenlerin ortak izni; farklıysa ya da bilinmiyorsa ilk bilineni, `mixed` ile. */
export function startMode(modes: (number | null | undefined)[]): { mode: number; mixed: boolean } {
  const known = modes.filter((m): m is number => typeof m === "number");
  if (!known.length) return { mode: 0o644, mixed: modes.length > 0 };
  const mixed = known.length !== modes.length || known.some((m) => m !== known[0]);
  return { mode: known[0] & 0o7777, mixed };
}

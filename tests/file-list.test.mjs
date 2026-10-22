// Dosya panellerinin sıralama ve seçim mantığı. Çalıştırma: npm test (Node 22.6+, TS dosyalarını doğrudan yükler).
import assert from "node:assert/strict";
import { sortEntries, toggleSort, clickSel, keySel } from "../src/lib/file-list.ts";

const e = (name, isDir, size = 0, mtime = 0) => ({ name, path: "/" + name, isDir, isLink: false, size, mtime, perms: "" });
const list = [e("b.txt", false, 300, 30), e("Proje 10", true, 0, 5), e("a.txt", false, 100, 10), e("proje 2", true, 0, 50), e("Çanta.log", false, 200, 20), e("c.txt", false, 100, 40)];
const names = (l) => l.map((x) => x.name);

assert.deepEqual(names(sortEntries(list, { key: "name", desc: false })), ["proje 2", "Proje 10", "a.txt", "b.txt", "c.txt", "Çanta.log"]);
assert.deepEqual(names(sortEntries(list, { key: "name", desc: true })), ["Proje 10", "proje 2", "Çanta.log", "c.txt", "b.txt", "a.txt"]);
// Boyut: klasörler üstte, eşit boyutta ada göre.
assert.deepEqual(names(sortEntries(list, { key: "size", desc: true })), ["proje 2", "Proje 10", "b.txt", "Çanta.log", "a.txt", "c.txt"]);
assert.deepEqual(names(sortEntries(list, { key: "size", desc: false })), ["proje 2", "Proje 10", "a.txt", "c.txt", "Çanta.log", "b.txt"]);
assert.deepEqual(names(sortEntries(list, { key: "mtime", desc: true })), ["proje 2", "Proje 10", "c.txt", "b.txt", "Çanta.log", "a.txt"]);
assert.deepEqual(toggleSort({ key: "name", desc: false }, "name"), { key: "name", desc: true });
assert.deepEqual(toggleSort({ key: "name", desc: false }, "mtime"), { key: "mtime", desc: true });

const order = ["/1", "/2", "/3", "/4", "/5"];
const empty= { selected: new Set(), anchor: null, cursor: null };
const ev = (o) => ({ shiftKey: false, ctrlKey: false, metaKey: false, altKey: false, ...o });
const sel = (s) => [...s.selected].sort();
// Çoklu seçim tuşu: macOS'ta Cmd, diğerlerinde Ctrl.
const mod = navigator.platform.toLowerCase().includes("mac") ? { metaKey: true } : { ctrlKey: true };

let s = clickSel(empty, ev({}), "/2", order);
s = clickSel(s, ev({ shiftKey: true }), "/4", order);
assert.deepEqual(sel(s), ["/2", "/3", "/4"]);
// Cmd/Ctrl ile ekle/çıkar
s = clickSel(s, ev(mod), "/3", order);
assert.deepEqual(sel(s), ["/2", "/4"]);
s = clickSel(s, ev(mod), "/5", order);
assert.deepEqual(sel(s), ["/2", "/4", "/5"]);
// Klavye
let k = keySel(empty, ev({ key: "ArrowDown" }), order);
assert.deepEqual([k.cursor, sel(k)], ["/1", ["/1"]]);
k = keySel(k, ev({ key: "ArrowDown", shiftKey: true }), order);
k = keySel(k, ev({ key: "ArrowDown", shiftKey: true }), order);
assert.deepEqual([k.cursor, sel(k)], ["/3", ["/1", "/2", "/3"]]);
k = keySel(k, ev({ key: "ArrowUp", shiftKey: true }), order);
assert.deepEqual(sel(k), ["/1", "/2"]);
k = keySel(k, ev({ key: "End" }), order);
assert.deepEqual([k.cursor, sel(k)], ["/5", ["/5"]]);
k = keySel(k, ev({ key: "ArrowDown" }), order);
assert.equal(k.cursor, "/5", "sonda kalır");
k = keySel(k, ev({ key: "a", ...mod }), order);
assert.equal(k.selected.size, 5);
k = keySel(k, ev({ key: "Escape" }), order);
assert.equal(k.selected.size, 0);
assert.equal(keySel(k, ev({ key: "Escape" }), order), null, "boş seçimde Esc işlenmez");
assert.equal(keySel(k, ev({ key: "x" }), order), null);
console.log("file-list: tüm testler geçti");

// Varsayılan uygulamayla açılacak dosyalar.
import { opensExternally } from "../src/lib/file-list.ts";
assert.equal(opensExternally("rapor.XLSX"), true);
assert.equal(opensExternally("veri.csv"), true);
assert.equal(opensExternally("a.tar.gz"), true);
assert.equal(opensExternally("nginx.conf"), false);
assert.equal(opensExternally("Makefile"), false);
assert.equal(opensExternally(".pdf"), false);

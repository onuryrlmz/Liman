// İzin penceresinin sekizli/bit dönüşümleri. Çalıştırma: npm test.
import assert from "node:assert/strict";
import { parseOctal, toOctal, toggle, startMode } from "../src/lib/perms.ts";

assert.equal(parseOctal("755"), 0o755);
assert.equal(parseOctal(" 0644 "), 0o644);
assert.equal(parseOctal("4755"), 0o4755);
assert.equal(parseOctal("000"), 0);
for (const bad of ["", "75", "789", "77777", "rwx", "-755", "7 5 5", "0o755"]) assert.equal(parseOctal(bad), null, bad);

assert.equal(toOctal(0o755), "755");
assert.equal(toOctal(0o7), "007");
assert.equal(toOctal(0o4755), "4755");
assert.equal(toOctal(0o1777), "1777");
assert.equal(toOctal(0o100644), "644", "dosya türü bitleri atılır");

assert.equal(toggle(0o644, 0o100, true), 0o744);
assert.equal(toggle(0o744, 0o100, false), 0o644);
assert.equal(toggle(0o644, 0o004, true), 0o644);

assert.deepEqual(startMode([0o755, 0o755]), { mode: 0o755, mixed: false });
assert.deepEqual(startMode([0o755, 0o644]), { mode: 0o755, mixed: true });
assert.deepEqual(startMode([null, 0o600]), { mode: 0o600, mixed: true });
assert.deepEqual(startMode([null]), { mode: 0o644, mixed: true });

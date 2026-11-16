// Oturum grubu ağacı. Çalıştırma: npm test
import assert from "node:assert/strict";
import { buildTree, parentOf, leafName, inGroup } from "../src/lib/groups.ts";

const shape = (nodes) => nodes.map((n) => (n.children.length ? [n.name, shape(n.children)] : n.name));
assert.deepEqual(shape(buildTree(["B", "A/x", "A", "A/x/1", "C/y"])), ["B", ["A", [["x", ["1"]]]], ["C", ["y"]]]);
assert.equal(buildTree(["A/x"])[0].children[0].path, "A/x");
assert.equal(parentOf("A/x/1"), "A/x");
assert.equal(parentOf("A"), null);
assert.equal(leafName("A/x/1"), "1");
assert.equal(inGroup("A/x", "A"), true);
assert.equal(inGroup("A", "A"), true);
assert.equal(inGroup("AB", "A"), false);

// Oturum grupları yolla yazılır: "Ankara/Web", "Ankara" grubunun alt grubudur.

export interface GroupNode {
  path: string;
  name: string;
  children: GroupNode[];
}

export const parentOf = (g: string): string | null => (g.includes("/") ? g.slice(0, g.lastIndexOf("/")) : null);
export const leafName = (g: string): string => g.slice(g.lastIndexOf("/") + 1);
/** `g`, `root` grubunun kendisi ya da alt grubu mu? */
export const inGroup = (g: string, root: string): boolean => g === root || g.startsWith(root + "/");

/** Düz grup listesinden ağaç; kardeşler listedeki sırayı korur, eksik üst gruplar eklenir. */
export function buildTree(groups: string[]): GroupNode[] {
  const roots: GroupNode[] = [];
  const byPath = new Map<string, GroupNode>();
  const node = (path: string): GroupNode => {
    let n = byPath.get(path);
    if (!n) {
      n = { path, name: leafName(path), children: [] };
      byPath.set(path, n);
      const parent = parentOf(path);
      (parent ? node(parent).children : roots).push(n);
    }
    return n;
  };
  groups.forEach(node);
  return roots;
}

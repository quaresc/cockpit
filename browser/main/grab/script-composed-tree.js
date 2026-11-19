// Ported from Orca (https://github.com/stablyai/orca, commit 49498c3), src/shared/browser/element-grab/browser-composed-tree-script.ts.
// MIT licence, Copyright (c) 2026 Lovecast Inc. Translated from TypeScript and adapted to cockpit's browser shell.

/**
 * Walks up through slots and shadow hosts, and lists the open shadow roots above an element. Each value is type-checked
 * first, because a page's named form controls can shadow DOM properties.
 */
export const COMPOSED_TREE_SCRIPT = `  function composedParent(element) {
    var slot = element.assignedSlot;
    if (slot && slot.localName === 'slot') return slot;
    var parent = element.parentNode;
    if (parent && parent.nodeType === 11) return parent.host || null;
    return parent && parent.nodeType === 1 ? parent : null;
  }

  // Scroll events don't leave a shadow root, so trackers listen on each open root above the target.
  function shadowRootsAbove(element) {
    var roots = [];
    for (var node = element; node; node = composedParent(node)) {
      var parent = node.parentNode;
      if (parent && parent.nodeType === 11 && parent.host) roots.push(parent);
    }
    return roots;
  }
`;

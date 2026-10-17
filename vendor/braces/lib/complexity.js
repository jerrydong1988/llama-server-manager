'use strict';

// Fixed bounds protect every entry point, including direct lib imports.
const MAX_DEPTH = 128;
const MAX_VISITS = 65536;

const reject = () => {
  const error = new SyntaxError('Brace expression exceeds the safe nesting or AST complexity limit');
  error.code = 'ERR_BRACES_COMPLEXITY';
  throw error;
};

exports.assertDepth = depth => {
  if (depth > MAX_DEPTH) reject();
};

exports.assertTree = root => {
  const pending = [{ node: root, depth: 0, next: -1 }];
  const active = new Set();
  let visits = 0;

  // Use an iterative walk. Only child edges matter; parser parent/prev
  // backlinks are intentional. Shared children are allowed, child cycles are not.
  while (pending.length > 0) {
    const frame = pending[pending.length - 1];
    const node = frame.node;
    if (frame.next === -1) {
      if (!node || typeof node !== 'object'
        || frame.depth > MAX_DEPTH || ++visits > MAX_VISITS
        || active.has(node) || Array.isArray(node.value)
        || (node.nodes && !Array.isArray(node.nodes))) {
        reject();
      }
      active.add(node);
      frame.next = 0;
    }
    if (node.nodes && frame.next < node.nodes.length) {
      pending.push({ node: node.nodes[frame.next++], depth: frame.depth + 1, next: -1 });
    } else {
      active.delete(node);
      pending.pop();
    }
  }
};

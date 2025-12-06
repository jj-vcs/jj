/**
 * Remark plugin to include markdown from external files.
 *
 * Usage in markdown:
 *   ::include{file="../path/to/file.md" start="<!-- BEGIN MARKDOWN-->"}
 *
 * Options:
 *   - file: Path to the file to include (relative to the markdown file)
 *   - start: Optional marker to start including from (content after this marker)
 *   - end: Optional marker to stop including at (content before this marker)
 */

import fs from 'node:fs';
import path from 'node:path';
import { visit } from 'unist-util-visit';
import { fromMarkdown } from 'mdast-util-from-markdown';
import { gfm } from 'micromark-extension-gfm';
import { gfmFromMarkdown } from 'mdast-util-gfm';

export default function remarkIncludeMarkdown(options = {}) {
  const { basePath = process.cwd() } = options;

  return (tree, file) => {
    const includes = [];

    // Find all ::include directives
    visit(tree, 'leafDirective', (node, index, parent) => {
      if (node.name !== 'include') return;

      const filePath = node.attributes?.file;
      if (!filePath) {
        console.warn('::include directive missing "file" attribute');
        return;
      }

      // Resolve the file path relative to the docs directory
      const resolvedPath = path.resolve(basePath, filePath);

      if (!fs.existsSync(resolvedPath)) {
        console.warn(`Include file not found: ${resolvedPath}`);
        return;
      }

      let content = fs.readFileSync(resolvedPath, 'utf-8');

      // Handle start marker
      const startMarker = node.attributes?.start;
      if (startMarker) {
        const startIndex = content.indexOf(startMarker);
        if (startIndex !== -1) {
          content = content.slice(startIndex + startMarker.length);
        }
      }

      // Handle end marker
      const endMarker = node.attributes?.end;
      if (endMarker) {
        const endIndex = content.indexOf(endMarker);
        if (endIndex !== -1) {
          content = content.slice(0, endIndex);
        }
      }

      content = content.trim();

      // Parse the included markdown into AST nodes with GFM support
      const includedTree = fromMarkdown(content, {
        extensions: [gfm()],
        mdastExtensions: [gfmFromMarkdown()],
      });

      // Store for later replacement (can't modify during visit)
      includes.push({ index, parent, nodes: includedTree.children });
    });

    // Replace directives with included content (in reverse to maintain indices)
    for (const { index, parent, nodes } of includes.reverse()) {
      parent.children.splice(index, 1, ...nodes);
    }
  };
}

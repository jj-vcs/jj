/**
 * Remark plugin to render YAML files as markdown tables.
 *
 * Usage in markdown:
 *   ::yaml-table{file="git-command-table.yml"}
 *
 * The YAML file should be an array of objects. Each object becomes a row,
 * and the keys become column headers.
 */

import fs from 'node:fs';
import path from 'node:path';
import yaml from 'js-yaml';
import { visit } from 'unist-util-visit';
import { fromMarkdown } from 'mdast-util-from-markdown';
import { gfm } from 'micromark-extension-gfm';
import { gfmFromMarkdown } from 'mdast-util-gfm';

export default function remarkYamlTable(options = {}) {
  const { basePath = process.cwd() } = options;

  return (tree, file) => {
    const replacements = [];

    // Find all ::yaml-table directives
    visit(tree, 'leafDirective', (node, index, parent) => {
      if (node.name !== 'yaml-table') return;

      const filePath = node.attributes?.file;
      if (!filePath) {
        console.warn('::yaml-table directive missing "file" attribute');
        return;
      }

      // Resolve the file path relative to the docs directory
      const resolvedPath = path.resolve(basePath, filePath);

      if (!fs.existsSync(resolvedPath)) {
        console.warn(`YAML file not found: ${resolvedPath}`);
        return;
      }

      const content = fs.readFileSync(resolvedPath, 'utf-8');
      let data;
      try {
        data = yaml.load(content);
      } catch (e) {
        console.warn(`Failed to parse YAML file: ${resolvedPath}`, e);
        return;
      }

      if (!Array.isArray(data) || data.length === 0) {
        console.warn(`YAML file should contain an array: ${resolvedPath}`);
        return;
      }

      // Get column headers from the first object's keys
      const headers = Object.keys(data[0]);

      // Build markdown table
      const headerRow = '| ' + headers.join(' | ') + ' |';
      const separatorRow = '| ' + headers.map(() => '---').join(' | ') + ' |';

      const dataRows = data.map((row) => {
        const cells = headers.map((header) => {
          let value = row[header] ?? '';
          // Convert to string and escape pipe characters
          value = String(value).trim().replace(/\|/g, '\\|');
          // Replace newlines with spaces for table cells
          value = value.replace(/\n/g, ' ');
          return value;
        });
        return '| ' + cells.join(' | ') + ' |';
      });

      const tableMarkdown = [headerRow, separatorRow, ...dataRows].join('\n');

      // Parse the table markdown into AST nodes
      const tableTree = fromMarkdown(tableMarkdown, {
        extensions: [gfm()],
        mdastExtensions: [gfmFromMarkdown()],
      });

      // Store for later replacement
      replacements.push({ index, parent, nodes: tableTree.children });
    });

    // Replace directives with table content (in reverse to maintain indices)
    for (const { index, parent, nodes } of replacements.reverse()) {
      parent.children.splice(index, 1, ...nodes);
    }
  };
}

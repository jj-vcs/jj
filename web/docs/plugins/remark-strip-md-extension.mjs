/**
 * Remark plugin to strip .md extensions from relative links and resolve paths.
 *
 * This allows markdown files to use links like [text](other-file.md) which
 * work on GitHub, while producing clean URLs like /other-file/ in the built site.
 *
 * Links are kept relative (not absolute) so they work with any base path
 * configuration. The key insight is that Starlight serves pages with trailing
 * slashes (e.g., /guides/divergence/), which browsers treat as directories.
 * For non-index files, we prepend ../ to counteract this behavior.
 *
 * Handles both inline links [text](url) and reference definitions [ref]: url
 */

import { visit } from 'unist-util-visit';
import path from 'node:path';

export default function remarkStripMdExtension() {
  return (tree, file) => {
    // Get the file's path relative to the content directory
    const filePath = file.history[0];
    const contentDocsMatch = filePath.match(/src\/content\/docs\/(.+)$/);
    const relativePath = contentDocsMatch[1];
    const currentDir = path.dirname(relativePath);

    // Check if current file is an index file
    const isIndex = path.basename(relativePath) === 'index.md';

    // Calculate directory depth for absolute link conversion
    const dirParts = currentDir === '.' ? [] : currentDir.split(path.sep);
    const depth = dirParts.length;
    visit(tree, ['link', 'definition'], (node) => {
      const url = node.url;
      if (!url || typeof url !== 'string') return;

      // Skip external links and anchors
      if (url.startsWith('http://') || url.startsWith('https://') || url.startsWith('#')) {
        return;
      }

      // Separate the path from hash/query
      const hashIndex = url.indexOf('#');
      const queryIndex = url.indexOf('?');
      let pathEnd = url.length;
      if (hashIndex !== -1) pathEnd = Math.min(pathEnd, hashIndex);
      if (queryIndex !== -1) pathEnd = Math.min(pathEnd, queryIndex);

      const linkPath = url.slice(0, pathEnd);
      const suffix = url.slice(pathEnd); // hash or query string

      // Only process links with .md extension in the path (not in hash/query)
      if (!linkPath.endsWith('.md')) {
        return;
      }

      // Strip .md extension
      const pathWithoutMd = linkPath.slice(0, -3);

      let resultPath;

      if (pathWithoutMd.startsWith('/')) {
        // Absolute link: convert to relative
        const targetPath = pathWithoutMd.slice(1); // Remove leading /
        // Non-index files are served at an extra level due to trailing slash
        const effectiveDepth = isIndex ? depth : depth + 1;
        const prefix = '../'.repeat(effectiveDepth);
        resultPath = prefix + targetPath;
      } else {
        // Relative link: prepend ../ for non-index files to counteract trailing slash
        if (isIndex) {
          resultPath = pathWithoutMd;
        } else {
          resultPath = '../' + pathWithoutMd;
        }
      }

      // Normalize the path to resolve ../ sequences
      // But keep it relative (don't let it become absolute)
      resultPath = path.normalize(resultPath).replace(/\\/g, '/');

      // Handle index file targets: foo/index -> foo
      resultPath = resultPath.replace(/\/index$/, '');

      // Add trailing slash for Starlight's URL structure
      if (!resultPath.endsWith('/')) {
        resultPath += '/';
      }

      node.url = resultPath + suffix;
    });
  };
}

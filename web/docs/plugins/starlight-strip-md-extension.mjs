/**
 * Starlight plugin that wraps the remark-strip-md-extension plugin.
 *
 * This wrapper is needed because Starlight overrides the markdown.remarkPlugins
 * configuration. By using a Starlight plugin, we ensure our remark plugin is
 * properly integrated into Starlight's markdown processing pipeline.
 */

import remarkStripMdExtension from './remark-strip-md-extension.mjs';

export default function starlightStripMdExtension() {
  return {
    name: 'starlight-strip-md-extension',
    hooks: {
      'config:setup'({ addIntegration }) {
        addIntegration({
          name: 'starlight-strip-md-extension-integration',
          hooks: {
            'astro:config:setup'({ updateConfig }) {
              updateConfig({
                markdown: {
                  remarkPlugins: [remarkStripMdExtension],
                },
              });
            },
          },
        });
      },
    },
  };
}

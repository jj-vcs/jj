import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

export const collections = {
	docs: defineCollection({
		// We don't use the regular `docsLoader` from starlight here, because that
		// doesn't allow us to configure the location of the markdown content files
		// enough. Instead, we use astro's more flexible `glob` loader directly. The
		// pattern is a simplified version of what starlight's `docsLoader` uses,
		// see: ../node_modules/@astrojs/starlight/loaders.ts
		loader: glob({
			base: "../docs",
			pattern: `**/[^_]*.{md,mdx}`,
		}),
		schema: docsSchema(),
	}),
};

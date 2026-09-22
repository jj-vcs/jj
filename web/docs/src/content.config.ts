import { defineCollection } from 'astro:content';
import { glob } from 'astro/loaders';
import { docsSchema } from '@astrojs/starlight/schema';

export const collections = {
	docs: defineCollection({
		loader: glob({
			base: '../../docs',
			pattern: '**/[^_]*.{markdown,mdown,mkdn,mkd,mdwn,md,mdx}',
		}),
		schema: docsSchema(),
	}),
};

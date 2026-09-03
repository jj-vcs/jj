// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import remarkDirective from 'remark-directive';
import remarkGfm from 'remark-gfm';
import remarkIncludeMarkdown from './plugins/remark-include-markdown.mjs';
import remarkYamlTable from './plugins/remark-yaml-table.mjs';
import starlightStripMdExtension from './plugins/starlight-strip-md-extension.mjs';
import { fileURLToPath } from 'node:url';

const docsDir = fileURLToPath(new URL('src/content/docs', import.meta.url));

// https://astro.build/config
export default defineConfig({
	site: 'https://docs.jj-vcs.dev',
	markdown: {
		remarkPlugins: [
			remarkGfm,
			remarkDirective,
			[remarkIncludeMarkdown, { basePath: docsDir }],
			[remarkYamlTable, { basePath: docsDir }],
		],
	},
	integrations: [
		starlight({
			plugins: [starlightStripMdExtension()],
			title: 'Jujutsu docs',
			favicon: "/images/jj-logo.svg",
			customCss: ['./src/styles/custom.css'],
			logo: {
				src: "./public/images/jj-logo.svg",
			},
			components: {
				ThemeSelect: './src/components/ThemeVersionSelect.astro',
			},
			markdown: {
				processedDirs: ['../../docs'],
			},
			social: [
				{
					icon: 'github',
					label: 'GitHub',
					href: 'https://github.com/jj-vcs/jj'
				},
				{
					icon: "discord",
					label: "Discord",
					href: "https://discord.gg/dkmfj3aGQN",
				},
			],
			sidebar: [
				{ label: 'Home', slug: 'index' },
				{
					label: 'Getting started',
					items: [
						{ label: 'Installation and setup', slug: 'getting-started/install-and-setup' },
						{ label: "Tutorial and bird's eye view", slug: 'getting-started/tutorial' },
						{ label: 'Working with Gerrit', slug: 'getting-started/gerrit' },
						{ label: 'Working with GitHub', slug: 'getting-started/github' },
						{ label: 'Working on Windows', slug: 'getting-started/windows' },
					],
					collapsed: true,
				},
				{ label: 'FAQ', slug: 'faq' },
				{ label: 'CLI reference', slug: 'cli-reference' },
				{ label: 'Testimonials', slug: 'testimonials' },
				{ label: 'Community-built tools', slug: 'community-tools' },
				{
					label: 'Concepts',
					items: [
						{ label: 'Working copy', slug: 'concepts/working-copy' },
						{ label: 'Bookmarks', slug: 'concepts/bookmarks' },
						{ label: 'Conflicts', slug: 'concepts/conflicts' },
						{ label: 'Operation log', slug: 'concepts/operation-log' },
						{ label: 'Glossary', slug: 'concepts/glossary' },
					],
					collapsed: true,
				},
				{
					label: 'Guides',
					items: [
						{ label: 'Divergent changes', slug: 'guides/divergence' },
						{ label: 'Multiple remotes', slug: 'guides/multiple-remotes' },
					],
					collapsed: true,
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Configuration', slug: 'reference/config' },
						{ label: 'Fileset language', slug: 'reference/filesets' },
						{ label: 'Revset language', slug: 'reference/revsets' },
						{ label: 'Templating language', slug: 'reference/templates' },
					],
					collapsed: true,
				},
				{
					label: 'Comparisons',
					items: [
						{ label: 'Git comparison', slug: 'comparisons/git-comparison' },
						{ label: 'Git command table', slug: 'comparisons/git-command-table' },
						{ label: 'Git compatibility', slug: 'comparisons/git-compatibility' },
						{ label: 'Jujutsu for Git experts', slug: 'comparisons/git-experts' },
						{ label: 'Sapling comparison', slug: 'comparisons/sapling-comparison' },
						{ label: 'Other related work', slug: 'comparisons/related-work' },
					],
					collapsed: true,
				},
				{
					label: 'Technical details',
					items: [
						{ label: 'Core tenets', slug: 'technical-details/core-tenets' },
						{ label: 'Architecture', slug: 'technical-details/architecture' },
						{ label: 'Concurrency', slug: 'technical-details/concurrency' },
						{ label: 'Conflicts', slug: 'technical-details/conflicts' },
					],
					collapsed: true,
				},
				{
					label: 'Contributing',
					items: [
						{ label: 'Guidelines and "How to...?"', slug: 'contributing/guidelines-and-how-to' },
						{ label: 'Code of conduct', slug: 'contributing/code-of-conduct' },
						{ label: 'Style guide', slug: 'contributing/style-guide' },
						{ label: 'Design docs', slug: 'contributing/design-docs' },
						{ label: 'Design doc blueprint', slug: 'contributing/design-doc-blueprint' },
						{ label: 'Releasing', slug: 'contributing/releasing' },
						{ label: 'Temporary voting for governance', slug: 'contributing/temporary-voting' },
						{ label: 'Governance', slug: 'contributing/governance' },
					],
					collapsed: true,
				},
				{
					label: 'Design docs',
					items: [
						{ label: 'git-submodules', slug: 'design/git-submodules' },
						{ label: 'git-submodule-storage', slug: 'design/git-submodule-storage' },
						{ label: 'JJ run', slug: 'design/run' },
						{ label: 'Sparse patterns v2', slug: 'design/sparse-v2' },
						{ label: 'Tracking branches', slug: 'design/tracking-branches' },
						{ label: 'Copy tracking and tracing', slug: 'design/copy-tracking' },
						{ label: 'Secure config', slug: 'design/secure-config' },
					],
					collapsed: true,
				},
				{ label: 'Roadmap', slug: 'roadmap' },
				{ label: 'Changelog', slug: 'changelog' },
			],
		}),
	],
});

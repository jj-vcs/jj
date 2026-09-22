// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import mermaid from 'astro-mermaid';
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
		mermaid(),
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
						{ label: 'Installation and setup', slug: 'install-and-setup' },
						{ label: "Tutorial and bird's eye view", slug: 'tutorial' },
						{ label: 'Working with Gerrit', slug: 'gerrit' },
						{ label: 'Working with GitHub', slug: 'github' },
						{ label: 'Working on Windows', slug: 'windows' },
					],
					collapsed: true,
				},
				{ label: 'FAQ', slug: 'faq' },
				{ label: 'CLI reference', slug: 'cli-reference' },
				{ label: 'Testimonials', slug: 'testimonials' },
				{ label: 'Community-built tools', slug: 'community_tools' },
				{
					label: 'Concepts',
					items: [
						{ label: 'Working copy', slug: 'working-copy' },
						{ label: 'Bookmarks', slug: 'bookmarks' },
						{ label: 'Conflicts', slug: 'conflicts' },
						{ label: 'Operation log', slug: 'operation-log' },
						{ label: 'Glossary', slug: 'glossary' },
					],
					collapsed: true,
				},
				{
					label: 'Guides',
					items: [
						{ label: 'CLI options for specifying revisions', slug: 'guides/cli-revision-options' },
						{ label: 'Divergent changes', slug: 'guides/divergence' },
						{ label: 'Multiple remotes', slug: 'guides/multiple-remotes' },
					],
					collapsed: true,
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Configuration', slug: 'config' },
						{ label: 'Fileset language', slug: 'filesets' },
						{ label: 'Revset language', slug: 'revsets' },
						{ label: 'Templating language', slug: 'templates' },
					],
					collapsed: true,
				},
				{
					label: 'Comparisons',
					items: [
						{ label: 'Git comparison', slug: 'git-comparison' },
						{ label: 'Git command table', slug: 'git-command-table' },
						{ label: 'Git compatibility', slug: 'git-compatibility' },
						{ label: 'Jujutsu for Git experts', slug: 'git-experts' },
						{ label: 'Sapling comparison', slug: 'sapling-comparison' },
						{ label: 'Other related work', slug: 'related-work' },
					],
					collapsed: true,
				},
				{
					label: 'Technical details',
					items: [
						{ label: 'Core tenets', slug: 'core_tenets' },
						{ label: 'Architecture', slug: 'technical/architecture' },
						{ label: 'Concurrency', slug: 'technical/concurrency' },
						{ label: 'Conflicts', slug: 'technical/conflicts' },
						{ label: 'Revset evaluation', slug: 'technical/revset-evaluation' },
					],
					collapsed: true,
				},
				{
					label: 'Contributing',
					items: [
						{ label: 'Guidelines and "How to...?"', slug: 'contributing' },
						{ label: 'Code of conduct', slug: 'code-of-conduct' },
						{ label: 'Style guide', slug: 'style_guide' },
						{ label: 'Design docs', slug: 'design_docs' },
						{ label: 'Design doc blueprint', slug: 'design_doc_blueprint' },
						{ label: 'Releasing', slug: 'releasing' },
						{ label: 'Temporary voting for governance', slug: 'governance/temporary-voting' },
						{ label: 'Governance', slug: 'governance/governance' },
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
						{ label: 'Managed config', slug: 'design/managed-config' },
						{ label: 'JJ converge (aka resolve-divergence)', slug: 'design/jj-converge-command' },
					],
					collapsed: true,
				},
				{ label: 'Development roadmap', slug: 'roadmap' },
				{ label: 'Changelog', slug: 'changelog' },
			],
		}),
	],
	vite: {
		resolve: {
			// The markdown content files live in `docs/` which is outside the vite
			// project root `web/`. Import statements in `docs/` will therefore not
			// resolve dependencies in `web/node_modules/`. This is specifically
			// relevant for `*.mdx` files, which use import statements and JSX syntax
			// to use components that don't map onto some markdown syntax, for example
			// the `<Tabs>` component. This `dedupe` option has a different purpose,
			// but can still be used to fix the problem. It forces dependency
			// resolution to start from the project root. see:
			// https://vite.dev/config/shared-options?utm_source=openai#resolve-dedupe
			dedupe: ['@astrojs/starlight'],
		},
	},
});

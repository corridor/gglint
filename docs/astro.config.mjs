// The gglint documentation site, built with Starlight (https://starlight.astro.build).
// Pages live in src/content/docs/ and are grouped the Diátaxis way: Start here (tutorials),
// Guides (how-to), Reference, and Concepts (explanation).
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://corridor.github.io',
  base: '/gglint',
  integrations: [
    starlight({
      title: 'gglint',
      description:
        'Keep the MCP servers of Claude Code, Codex, Cursor, Gemini CLI and other coding agents in sync, and secrets out of their config files.',
      favicon: '/favicon.svg',
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/corridor/gglint' }],
      editLink: { baseUrl: 'https://github.com/corridor/gglint/edit/main/docs/' },
      customCss: ['./src/styles/custom.css'],
      sidebar: [
        {
          label: 'Start here',
          items: [
            { label: 'Quick start', slug: 'start/quick-start' },
            { label: 'Install', slug: 'start/install' },
          ],
        },
        {
          label: 'Guides',
          items: [
            { label: 'Sync MCP servers across agents', slug: 'guides/sync-mcp-servers' },
            { label: 'Keep secrets out of agent files', slug: 'guides/keep-secrets-out' },
            { label: 'Check before every commit', slug: 'guides/pre-commit' },
            { label: 'Check every pull request', slug: 'guides/github-action' },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Command line', slug: 'reference/cli' },
            {
              label: 'Rules',
              items: [
                { label: 'Overview', slug: 'reference/rules' },
                { label: 'missing-server', slug: 'reference/rules/missing-server' },
                { label: 'missing-config', slug: 'reference/rules/missing-config' },
                { label: 'different-server', slug: 'reference/rules/different-server' },
                { label: 'renamed-server', slug: 'reference/rules/renamed-server' },
                { label: 'inline-secret', slug: 'reference/rules/inline-secret' },
                { label: 'inline-credential', slug: 'reference/rules/inline-credential' },
                { label: 'local-file', slug: 'reference/rules/local-file' },
              ],
            },
            { label: 'Supported agents', slug: 'reference/agents' },
            { label: 'Output formats', slug: 'reference/output-formats' },
            { label: 'GitHub Action', slug: 'reference/github-action' },
          ],
        },
        {
          label: 'Concepts',
          items: [
            { label: 'How servers are matched', slug: 'concepts/matching' },
            { label: 'How fixing works', slug: 'concepts/fixing' },
            { label: 'How secrets are found', slug: 'concepts/secrets' },
            { label: 'Security', slug: 'concepts/security' },
            { label: 'Limitations', slug: 'concepts/limitations' },
          ],
        },
        {
          label: 'Project',
          items: [
            { label: 'Contributing', slug: 'project/contributing' },
            { label: 'Changelog', link: 'https://github.com/corridor/gglint/blob/main/CHANGELOG.md' },
          ],
        },
      ],
    }),
  ],
});

// The gglint documentation site, built with Starlight (https://starlight.astro.build).
// The story: gglint is a linter for the files that set up coding agents, with two checks today.
// Pages live in src/content/docs/: Start here, one page per check, ways to run it automatically,
// Reference, and About.
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://corridor.github.io',
  base: '/gglint',
  // Pages that were merged into others keep working.
  redirects: {
    '/guides/sync-mcp-servers': '/gglint/checks/mcp-servers/',
    '/guides/keep-secrets-out': '/gglint/checks/secrets/',
    '/concepts/matching': '/gglint/checks/mcp-servers/#how-it-decides-two-servers-are-the-same',
    '/concepts/fixing': '/gglint/checks/mcp-servers/#fixing-it',
    '/concepts/secrets': '/gglint/checks/secrets/',
    '/concepts/limitations': '/gglint/checks/mcp-servers/#how-it-decides-two-servers-are-the-same',
    '/concepts/security': '/gglint/about/security/',
    '/reference/github-action': '/gglint/guides/github-action/#options',
  },
  integrations: [
    starlight({
      title: 'gglint',
      description:
        'A linter for coding-agent setup: gglint checks the files that set up Claude Code, Codex, Cursor and 18 more for drifted MCP servers and leaked secrets.',
      favicon: '/favicon.svg',
      logo: { light: './src/assets/logo-light.svg', dark: './src/assets/logo-dark.svg' },
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/corridor/gglint' }],
      editLink: { baseUrl: 'https://github.com/corridor/gglint/edit/main/docs/' },
      customCss: [
        '@fontsource-variable/geist',
        '@fontsource-variable/geist-mono',
        './src/styles/custom.css',
      ],
      components: { PageTitle: './src/components/PageTitle.astro' },
      // Code is always a dark terminal, in both themes.
      expressiveCode: {
        themes: ['github-dark-default'],
        useStarlightUiThemeColors: false,
        styleOverrides: {
          borderRadius: '0px',
          borderColor: 'var(--gg-line)',
          codeBackground: '#0a0a0a',
          codeFontFamily: 'var(--__sl-font-mono)',
          uiFontFamily: 'var(--__sl-font-mono)',
          frames: {
            shadowColor: 'transparent',
            editorTabBarBackground: '#161616',
            editorActiveTabBackground: '#0a0a0a',
            editorActiveTabIndicatorTopColor: '#d4ff3a',
            editorTabBarBorderBottomColor: '#2e2e2e',
            terminalTitlebarBackground: '#161616',
            terminalTitlebarBorderBottomColor: '#2e2e2e',
            terminalBackground: '#0a0a0a',
            frameBoxShadowCssValue: 'none',
          },
        },
      },
      sidebar: [
        {
          label: 'Start here',
          items: [
            { label: 'Quick start', slug: 'start/quick-start' },
            { label: 'Install', slug: 'start/install' },
          ],
        },
        {
          label: 'Checks',
          items: [
            { label: 'MCP servers', slug: 'checks/mcp-servers' },
            { label: 'Secrets', slug: 'checks/secrets' },
            { label: 'See what an agent sends', slug: 'checks/watch' },
          ],
        },
        {
          label: 'Run automatically',
          items: [
            { label: 'Before every commit', slug: 'guides/pre-commit' },
            { label: 'On every pull request', slug: 'guides/github-action' },
          ],
        },
        {
          label: 'Rules',
          items: [
            { label: 'All rules', slug: 'reference/rules' },
            {
              label: 'MCP servers',
              items: [
                { label: 'missing-server', slug: 'reference/rules/missing-server' },
                { label: 'missing-config', slug: 'reference/rules/missing-config' },
                { label: 'different-server', slug: 'reference/rules/different-server' },
                { label: 'renamed-server', slug: 'reference/rules/renamed-server' },
              ],
            },
            {
              label: 'Secrets',
              items: [
                { label: 'inline-secret', slug: 'reference/rules/inline-secret' },
                { label: 'inline-credential', slug: 'reference/rules/inline-credential' },
                { label: 'local-file', slug: 'reference/rules/local-file' },
              ],
            },
          ],
        },
        {
          label: 'Reference',
          items: [
            { label: 'Command line', slug: 'reference/cli' },
            { label: 'Supported agents', slug: 'reference/agents' },
            { label: 'Output formats', slug: 'reference/output-formats' },
          ],
        },
        {
          label: 'About',
          collapsed: true,
          items: [
            { label: 'Security', slug: 'about/security' },
            { label: 'Contributing', slug: 'project/contributing' },
            { label: 'Changelog', link: 'https://github.com/corridor/gglint/blob/main/CHANGELOG.md' },
          ],
        },
      ],
    }),
  ],
});

# gglint documentation

The source of gglint's documentation site, built with [Starlight](https://starlight.astro.build)
and published to <https://corridor.github.io/gglint/> from `main` by
[`.github/workflows/docs.yml`](../.github/workflows/docs.yml).

```sh
npm ci          # Node.js 22.23 or later
npm run dev     # serve the site with live reload
npm run build   # build it into dist/, as CI does
```

## Where pages go

Pages are Markdown (`.md`) or MDX (`.mdx`, for components such as tabs) files in
`src/content/docs/`. They're grouped the [Diátaxis](https://diataxis.fr) way, by what the
reader is doing:

| Folder        | Kind          | For a reader who wants to                       |
| ------------- | ------------- | ----------------------------------------------- |
| `start/`      | Tutorials     | get gglint running for the first time           |
| `guides/`     | How-to guides | do one task, such as checking pull requests     |
| `reference/`  | Reference     | look up a command, a rule, an agent or a format |
| `concepts/`   | Explanation   | understand how and why gglint works as it does  |
| `project/`    | Project       | contribute                                      |

A new page also needs an entry in the `sidebar` of [`astro.config.mjs`](astro.config.mjs).
Links between pages start with the site's base, `/gglint/`.

## The look

Ink, paper and one lime signal colour, set in Geist and Geist Mono: hairline rules, square
corners, no gradients. The theme is [`src/styles/custom.css`](src/styles/custom.css), the
landing page is [`src/components/Landing.astro`](src/components/Landing.astro), and rule pages
open with [`RuleSpec`](src/components/RuleSpec.astro).

`demo.svg`, the terminal animation, stays in this folder because the README links to it here.

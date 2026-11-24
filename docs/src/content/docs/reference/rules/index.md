---
title: Rules
description: Every problem gglint reports, grouped by check, and whether --fix fixes it.
---

Every problem gglint reports belongs to a rule, and every rule belongs to one of the two checks.
A problem fails the check. A warning, such as a secret in a file git ignores, doesn't.

| Rule                                                              | Check   | What it catches                                     | `--fix`             |
| ----------------------------------------------------------------- | ------- | --------------------------------------------------- | ------------------- |
| [`missing-server`](/gglint/reference/rules/missing-server/)       | MCP     | A server one agent has and another doesn't          | Adds it             |
| [`missing-config`](/gglint/reference/rules/missing-config/)       | MCP     | An agent with a folder but no MCP config            | Creates the file    |
| [`different-server`](/gglint/reference/rules/different-server/)   | MCP     | The same server with another version or arguments   | Makes them match    |
| [`renamed-server`](/gglint/reference/rules/renamed-server/)       | MCP     | The same server under another name                  | Reports only        |
| [`inline-secret`](/gglint/reference/rules/inline-secret/)         | Secrets | A token in a known format                           | Uses an env variable |
| [`inline-credential`](/gglint/reference/rules/inline-credential/) | Secrets | A password or key under a name, in a URL or command | Uses an env variable |
| [`local-file`](/gglint/reference/rules/local-file/)               | Secrets | A personal agent file that isn't gitignored         | Adds to `.gitignore` |

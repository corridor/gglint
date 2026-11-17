#!/usr/bin/env bash
# Runs gglint for the GitHub Action: annotations in the log, then the files the later steps
# upload or post. Reports only; nothing in the repository is changed.
#
# Inputs, as environment variables: INPUT_COMMAND (secrets, mcp or all), INPUT_PATH,
# INPUT_VERSION, INPUT_FAIL_ON_PROBLEMS, INPUT_COMMENT and INPUT_SARIF. GGLINT_BIN runs that
# program instead of `uvx gglint@VERSION`, for testing.

set -uo pipefail

command_input="${INPUT_COMMAND:-all}"
path="${INPUT_PATH:-.}"
version="${INPUT_VERSION:-}"
output="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/gglint"

gglint() {
  if [ -n "${GGLINT_BIN:-}" ]; then
    "$GGLINT_BIN" "$@"
  elif [ -n "$version" ] && [ "$version" != "latest" ]; then
    uvx --quiet "gglint@${version}" "$@"
  else
    uvx --quiet gglint "$@"
  fi
}

case "$command_input" in
  secrets) commands=(secrets) ;;
  mcp) commands=(mcp) ;;
  all) commands=(secrets mcp) ;;
  *)
    echo "::error::command must be secrets, mcp or all, not '$command_input'"
    exit 2
    ;;
esac

rm -rf "$output"
mkdir -p "$output/sarif"
sections="$output/sections.md"
: >"$sections"

status=0
for command in "${commands[@]}"; do
  # A repository with no MCP config files has nothing to compare: gglint says so and exits
  # with 1, which isn't a problem to report here.
  if [ "$command" = mcp ] && ! gglint mcp --servers "$path" >/dev/null 2>&1; then
    echo "::notice::gglint: no MCP config files found, so there's nothing to compare"
    continue
  fi

  # The annotations go to the log on standard output, where GitHub reads them.
  gglint "$command" --format github "$path" || status=1

  if [ "${INPUT_SARIF:-false}" = true ]; then
    gglint "$command" --format sarif "$path" >"$output/sarif/gglint-$command.sarif" 2>/dev/null || true
  fi
  if [ "${INPUT_COMMENT:-false}" = true ]; then
    {
      gglint "$command" --format markdown "$path" 2>/dev/null || true
      echo
    } >>"$sections"
  fi
done

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  if [ "${INPUT_SARIF:-false}" = true ] && [ -n "$(ls -A "$output/sarif")" ]; then
    echo "sarif-directory=$output/sarif" >>"$GITHUB_OUTPUT"
  fi
  if [ "${INPUT_COMMENT:-false}" = true ] && [ -s "$sections" ]; then
    {
      echo "<!-- gglint -->"
      cat "$sections"
      echo "<sub>[gglint](https://github.com/corridor/gglint): checks the MCP servers and secrets of every coding agent in this repository. It only reports.</sub>"
    } >"$output/comment.md"
    echo "comment-file=$output/comment.md" >>"$GITHUB_OUTPUT"
  fi
fi

if [ "${INPUT_FAIL_ON_PROBLEMS:-true}" = false ]; then
  exit 0
fi
exit "$status"

#!/usr/bin/env bash
# Posts the findings as one pull request comment, and updates that comment on later pushes.
# Needs GH_TOKEN, REPOSITORY, PULL_REQUEST and COMMENT_FILE in the environment.

set -euo pipefail

comments="repos/${REPOSITORY}/issues/${PULL_REQUEST}/comments"
existing=$(gh api --paginate "$comments" \
  --jq '.[] | select(.body | startswith("<!-- gglint -->")) | .id' | head -n 1 || true)

if [ -n "$existing" ]; then
  # Also when the problems are gone, so the comment doesn't go on reporting them.
  gh api --method PATCH "repos/${REPOSITORY}/issues/comments/${existing}" \
    --field "body=@${COMMENT_FILE}" >/dev/null
elif grep -q '❌' "$COMMENT_FILE"; then
  # A pull request with no problems gets no comment.
  gh api --method POST "$comments" --field "body=@${COMMENT_FILE}" >/dev/null
fi

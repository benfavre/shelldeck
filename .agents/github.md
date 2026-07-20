# GitHub operations

- Use the local GitHub CLI (`gh`) for **all** GitHub reads and writes, including
  pull requests, issues, reviews, comments, checks, and releases.
- Never use a Codex GitHub connector/app/MCP tool for this project. Connector
  authentication is independent from the local CLI and can attribute actions
  to the wrong GitHub account.
- Before any GitHub mutation, run `gh auth status` and `gh api user --jq .login`.
  The expected actor is `pedrokarim`; if another account is active, stop and
  ask the user instead of performing the operation.
- The repository owner shown in a URL (for example `benfavre/bext`) is not the
  action author. Verify the resulting PR/issue `author.login` after creation.

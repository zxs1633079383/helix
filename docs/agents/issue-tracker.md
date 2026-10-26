# Issue tracker: GitLab (self-hosted)

Issues and PRDs for this repo live as GitLab issues on the self-hosted instance
**`git.yundiz.com`**, project **`cses/go/helix`**. Use the
[`glab`](https://gitlab.com/gitlab-org/cli) CLI for all operations.

> **Self-hosted + HTTP note.** This instance is served over plain HTTP, and
> `glab` defaults to HTTPS. Auth was configured with
> `glab auth login --hostname git.yundiz.com --api-protocol http --git-protocol http`.
> If `glab` ever reports "no known GitLab host" or a TLS error, re-check that the
> `git.yundiz.com` host in `~/Library/Application Support/glab-cli/config.yml`
> has `api_protocol: http`.

## Conventions

- **Create an issue**: `glab issue create --title "..." --description "..."`. Use a heredoc for multi-line descriptions. Pass `--description -` to open an editor.
- **Read an issue**: `glab issue view <number> --comments`. Use `-F json` for machine-readable output.
- **List issues**: `glab issue list -F json` with appropriate `--label` filters.
- **Comment on an issue**: `glab issue note <number> --message "..."`. GitLab calls comments "notes".
- **Apply / remove labels**: `glab issue update <number> --label "..."` / `--unlabel "..."`. Multiple labels can be comma-separated or by repeating the flag. GitLab auto-creates a project label the first time it is applied, so the five triage labels do not need to be pre-created.
- **Close**: `glab issue close <number>`. `glab issue close` does not accept a closing comment, so post the explanation first with `glab issue note <number> --message "..."`, then close.
- **Merge requests**: GitLab calls PRs "merge requests". Use `glab mr create`, `glab mr view`, `glab mr note`, etc. — the same shape as `gh pr ...` with `mr` in place of `pr` and `note`/`--message` in place of `comment`/`--body`.

Infer the repo from `git remote -v` — `glab` does this automatically when run inside this clone (`origin` → `http://git.yundiz.com/cses/go/helix.git`).

## Merge requests as a triage surface

**MRs as a request surface: no.** _(This is an internal repo, not an open-source
project that receives feature requests as external MRs. `/triage` only processes
issues here. Set to `yes` if that changes.)_

## When a skill says "publish to the issue tracker"

Create a GitLab issue with `glab issue create`.

## When a skill says "fetch the relevant ticket"

Run `glab issue view <number> --comments`.

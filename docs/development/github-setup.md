# GitHub repository setup

Checklist for maintainers when the repository is first connected to GitHub. The workflows in `.github/` assume these settings.

## Before the first push

- **Confirm the private research is ignored** ([ADR-0015](../adr/0015-clean-room-policy.md)). `docs/capcut-audit/` and `docs/capcut-rea/` are listed in `.gitignore`. After `git init`, and before the first commit, check:

  ```bash
  git check-ignore -v docs/capcut-audit docs/capcut-rea
  ```

  Both paths must be reported with their `.gitignore` rule, and `git status --short` must not list them. CI also fails if any file from these folders is ever committed.
- **Sharing research with collaborators** needs a separate private repository. Redact personal data first: `docs/capcut-audit/screenshots/001-home.png` and `088-return-home.png` (plus `088-return-home.txt`) show personal CapCut projects, four export transcripts (`051`, `055`, `056`, `058`) contain an absolute local path with the user name, and settings captures `004` to `007` may show local paths in the image.
- **Confirm the license** in [ADR-0014](../adr/0014-licensing.md) (`MIT OR Apache-2.0`, copyright "The FossCut Contributors") or change it before the first public commit.
- **Sign off commits from the start** (`git commit -s`), since the DCO check runs on every pull request.

## Repository settings

| Setting | Value | Why |
|---|---|---|
| Default branch | `main` | Workflows trigger on `main` |
| Merge methods | Squash merging only, default message "Pull request title" | Conventional titles become the commit history that git-cliff turns into release notes |
| Automatically delete head branches | On | Keeps the branch list clean |
| Private vulnerability reporting | On (Security settings) | Used by `SECURITY.md` and by the Code of Conduct for private reports |
| Dependabot alerts and security updates | On | `.github/dependabot.yml` handles version updates |
| Actions, workflow permissions | Read repository contents by default; do not allow Actions to approve pull requests | Workflows request write access only where needed (release) |
| Actions, fork pull request workflows | Require approval for first-time contributors | Limits abuse of CI minutes |

## Branch protection (ruleset on `main`)

A pull request must not merge while any test or check fails ([ADR-0016](../adr/0016-testing-strategy.md)). Create a branch ruleset (Settings, Rules, Rulesets, New branch ruleset) targeting the default branch, with enforcement **Active**:

- **Require a pull request before merging.** With one maintainer, set required approvals to 0 (GitHub does not let authors approve their own pull requests); raise it to 1 and dismiss stale approvals on new commits once there are two maintainers.
- **Require status checks to pass**, with "Require branches to be up to date before merging" on, and these checks:

  | Check | Workflow | Covers |
  |---|---|---|
  | `CI result` | CI | Every CI job: docs and ADRs, Rust format, lints and docs, schemas, Rust tests on Linux, macOS and Windows, documentation tests, test coverage minimums, the WebAssembly build, dependency licenses and sources, and the UI checks |
  | `Conventional title` | Pull request checks | Pull request title |
  | `DCO sign-off` | Pull request checks | Sign-off on every commit |

  `CI result` fails when any CI job fails or is cancelled, so new jobs added to its `needs` list become required without changing the ruleset. Jobs skipped because a part of the project does not exist yet count as passing. The check names appear in the picker after the workflows have run once on a pull request.
- **Require linear history; block force pushes and deletion.**
- **Bypass list:** empty. Bypass only in emergencies, by temporarily adding a maintainer.

`cargo-deny advisories` (Dependency advisories workflow) is deliberately not required: a newly published advisory should be fixed in its own pull request, not block unrelated ones.

The same ruleset from the command line (needs an admin token):

```bash
gh api --method POST repos/fossworld/fosscut/rulesets --input docs/development/main-ruleset.json
```

## Labels

Create `bug`, `enhancement` and `task`, used by the issue templates.

## Optional

- **GitHub Discussions** for questions and showcase posts, keeping issues for actionable work.
- **CODEOWNERS** once there are several maintainers, for example to require a maintainer of `crates/fosscut-model` on format changes.
- **A dedicated contact address** for conduct reports, added to `CODE_OF_CONDUCT.md`, if GitHub private reporting is not enough.

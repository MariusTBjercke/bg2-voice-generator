# Local execution plans

This folder holds **machine-readable implementation plans** used during development:
`progress.json`, per-item markdown, handoff prompts, and audit artifacts.

## Convention

- Plans live under `docs/plans/<plan-name>/` on your machine only.
- **Nothing under this folder is published to GitHub** (see root `.gitignore`).
- Shared, durable context belongs in `docs/testing/`, `README.md`, and `AGENTS.md`
  instead.

When starting a new plan locally, create a subdirectory here and keep progress
files out of git. Do not add exceptions to `.gitignore` for plan content.

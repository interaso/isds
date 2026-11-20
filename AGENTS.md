# AGENTS.md

Read and follow [CONTRIBUTING.md](CONTRIBUTING.md). It is the source of truth for development setup, verification commands, API and protocol conventions, live-test safety, and licensing. Keep shared contributor guidance there rather than duplicating it in this file.

## Coding-agent workflow

- Inspect the relevant code and documentation before editing, and make the smallest focused change that satisfies the request.
- Run the applicable offline checks from `CONTRIBUTING.md` before finishing, and do not skip or weaken failing checks.
- Do not run live tests unless the user explicitly requests them. Live tests must use only the official ISDS test environment, never production.
- Never commit credentials, `.env` files, or other secrets.
- Do not publish releases or mutate Git history unless the user explicitly requests it.
- Update `CONTRIBUTING.md` when shared development guidance changes, and keep this file consistent with it.

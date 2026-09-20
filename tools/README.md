# Local Jev replay review

These development tools stay outside `src/strategy/`, the only folder mm-cli submits.
Node is required; no additional packages are needed. The analyzer reuses the official
decoder and payload geometry in the existing `.local-visualizer/` folder.

```sh
node tools/analyze-replay.mjs --self-test
node tools/analyze-replay.mjs                       # latest replay; local only
node tools/analyze-replay.mjs logs/log.mmgl --team b
node tools/analyze-replay.mjs --send                # one Jev request
node tools/analyze-replay.mjs --send --gateway      # one Jev request through Vercel
```

Sending requires a project-authorized key in `MECHMANIA_TYPESAFE_API_KEY`, injected
into the process environment. Never put the key in Rust, this file, or a replay.
For Vercel, use `--gateway` and inject `MECHMANIA_AI_GATEWAY_API_KEY` instead.
The helper does not read Infisical, switch profiles, or search other projects for keys.

Each run writes a new folder under `logs/analysis/` containing computed metrics,
the exact request payload, and a Markdown report. A successful `--send` also saves
the validated Jev response, returned model version, usage, and request duration.
The report distinguishes local-only analysis from an attempted call without a validated result.

Jev selects a predefined experiment; it does not generate a prose explanation or code.
Treat the selection as a hypothesis. Inspect the replay, change one bot behavior,
and compare match outcomes against saved opponents before keeping it. Self-play alone
does not establish strength. Metrics describe completed states, not intent or causality.

API: https://docs.typesafe.ai/api. Pinned model: `jev-1.13.0`.
Vercel: https://vercel.com/docs/ai-gateway/modalities/evaluation, model `typesafe-ai/jev`.
The Gateway alias is not version-pinned. Its experimental SDK v4 wire protocol is
called with built-in fetch; update this adapter if the protocol changes.

## Local strategy benchmark

After `cargo build --release`, run:

```sh
python3 tools/benchmark.py my-experiment clankerbot-v1 baseline
```

Each opponent names an existing binary in `.mm/opponents/`. The helper plays both
sides with engine time limits enabled, rejects bot errors, and writes replays and
a result JSON under `logs/`. Use a fresh experiment name to preserve previous runs.
Synthetic opponents are local strategy variants, not downloaded competitor bots.

## Training roster

For 43 configurable sparring profiles across seven categories, see
[`training/README.md`](../training/README.md). Run every profile on both sides:

```sh
python3 tools/train.py run --label my-candidate
```

The existing release candidate is frozen for each run. Results, category scores,
compressed replays and source/binary snapshots are saved under `logs/training/`.
The replay analyzer accepts both `.mmgl` and `.mmgl.gz` files.

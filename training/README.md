# Local sparring roster

94 opponents plus one exact-v12 self-play control, eleven categories. Everything runs locally inside the
normal game engine with its compute limits enabled. No external model or API is
called during a match. These tools are outside `src/strategy/` and are not submitted.

```sh
# View every preset; filters may be repeated.
python3 tools/train.py list
python3 tools/train.py list --category healing --category competitor

# Build all named executables in .mm/training/bots/.
python3 tools/train.py build

# Run the existing release bot against every opponent, once per side.
python3 tools/train.py run --label my-candidate

# Fast focused run, or compare an older saved release.
python3 tools/train.py run --category healing --label heal-check
python3 tools/train.py run --profile approx-potatoes --profile approx-noeyedeer \
  --candidate .mm/opponents/clankerbot-v5 --label v5-check
```

The runner **does not rebuild the candidate**. Build the root release bot with
`~/.cargo/bin/cargo build --release --locked` after changing your strategy, then run
it. Training builds use cached dependencies and their lockfile; on a fresh machine
run `~/.cargo/bin/cargo fetch --manifest-path training/Cargo.toml --locked` first.
The local engine must exist at `.mm/bin/mm-engine`.

| Category | Presets |
|---|---|
| rush | zero-miner all-in, one-miner rush, two-miner rush, rush with medic |
| healing | healer swarm, healer ball, four-medic escort, healer-only stress |
| economy | four-miner balanced, eight-miner factory, greed then army, economy turtle |
| objective | single guard, double guard, mass escort, payload camper |
| targeting | healer hunter, miner raider, weakest finisher, splash hunter |
| movement | dense ball, wide formation, long range, strafing skirmish |
| adversarial | twelve Team Name stress variants, three support/triage controls, six additional pressure formations |
| independent | Fable 5.1 and Cursor Grok 4.6 separately generated controllers |
| whitebox | six Fable, six Grok, and eight local variants designed with the full v12 source |
| control | exact frozen v12 self-play |
| competitor | approximations of Potatoes, noeyedeer, Syntax Terror, DIBSFA, Gang, terryduan-chn; economy-base-raider and healing-finisher-escort stress variants; three Janice-style formations; three Gang v9 delayed-push variants; three Gang v13 composition checks; two additional Janice stress variants |

`roster.json` holds the knobs. Add or edit a profile and the next run rebuilds the
training executable. Miner/healer counts are production targets: the bot normally keeps
existing units, replaces losses, and can fail to reach a target under pressure.
`mobilize: true` instead retires its miners at `switch_tick` and requires
`late_miners: 0`, leaving slots for combat units before paid builds close.
Retirement waits while an enemy fighter is within possible shot/splash reach: the
current engine can panic if a miner dies to a shot and self-destructs in the same tick.
`heal_every` is fighters per healer; zero uses a fixed healer target.
`guards: 32` makes every fighter an objective escort. `spacing` controls repulsion
strength, not an absolute distance. `range` is a fraction of maximum firing range;
terrain and guards can override it. Target priorities apply to exposed enemies.
`raid: true` sends fighters toward the enemy deposit instead of the payload; pair
it with `guards: 0` for a base attack.

Competitor approximations use the observed compositions and broad behavior in
`logs/scouting/tournament-5-review.md` and the newer tournament 6/7 replays; we do not have opponents' private source.
Departure delays and spacing strengths are modeling choices, not recovered code.
The original 32 presets retain their v4-based combat behavior. The new Janice-style profiles enable a separate `healing_ball` behavior on that same implementation.
The newer base-raider targets the enemy deposit immediately: match 65 showed that Gang raided our base while our original approximation waited at home. The eight-healer finisher matches the observed support cap in match 93 but uses hypothetical targeting. The original 30 profiles remain unchanged for comparisons.
They expose many matchup weaknesses but cannot reproduce every independent AI.
The healer-only preset is intentionally weak and should not inflate confidence.

The Janice-style profiles (`approx-janice-escort`, `approx-janice-focus`, and
`approx-janice-tight`) reproduce the observed 3-miner, 10-fighter, 4-healer opening
and aim to grow toward 20 fighters and 9 healers. `healing_ball: true` adds early
fighter production, wounded retreats toward medics, medics moving behind their
patients, and alternative legal shots. Their short pursuit range keeps the army
near the payload. Target priorities and spacing differ to test uncertainty in
Janice's private implementation. The [scouting report](../logs/scouting/refresh-20260919T062453Z/janice-review.md)
records the replay evidence; these are approximations, not cloned code.

```sh
python3 tools/train.py run --label janice-check \
  --profile approx-janice-escort --profile approx-janice-focus --profile approx-janice-tight
```

The Gang v9 profiles (`approx-gang-late-escort`, `approx-gang-late-focus`, and
`approx-gang-early-push`) open with 8 miners, 8 fighters, and 1 healer, then defend
spread posts near home while adding healing support. Medics follow nearby wounded
fighters instead of leaving the home defense to follow the payload. At tick 5835
(5200 for the early variant), miners self-destruct to release their slots and the
army advances; production targets up to 8 healers. This models the eight healthy
miners disappearing together at tick 5836 in match 381. Defensive posts use the
current fixed map; targeting, exact post assignments, and early timing are modeling
choices. The original 35 presets are unchanged. The approximation's side advantage
can differ from the real match, so local wins do not establish a live counter.

```sh
python3 tools/train.py run --label gang-check \
  --profile approx-gang-late-escort --profile approx-gang-late-focus --profile approx-gang-early-push
```

The newer `approx-gang13-*` profiles use the observed 8-miner/6-fighter/3-healer
opening, allow two guards to contest after tick 1500, and keep a reserve near home
until the pre-endgame transition. `staged_push` enables the reserve and
`opening_healers` controls the initial support count. These profiles are **composition
checks, not validated recreations**: v10 beats all six games, despite losing to the
real Gang v13. Do not use wins over them as evidence that a live counter works.
`approx-janice-medic-hunter` and `approx-janice-mass` retain Janice's observed opening
while stressing exposed-medics and stronger payload coverage; their tactics are
hypotheses, not recovered private code. V10 loses two of their four games.
The first 38 profile objects remain unchanged; the shared safe-retirement fix above
requires a fresh baseline when comparing with older recorded runs.

Each run saves `logs/training/<timestamp>-<label>/` with:

- `report.md`: totals, category breakdowns and both-side results.
- `results.json`: winner, reason, tick, errors, opponent compositions and replay paths.
- `manifest.json`, `source/`, `bin/`: exact roster, source snapshots, binaries and hashes.
- `.mmgl.gz` replays and separate engine/bot error logs. Interrupted runs retain completed results.

Matches run sequentially to avoid CPU-budget contention. Crashes, stderr, missing
results and timeouts count as **errors**, not wins. Replays are compressed after
each game. Analyze directly or decompress a copy for the visualizer:

```sh
node tools/analyze-replay.mjs logs/training/RUN/healer-swarm-a.mmgl.gz --team a
gzip -dk logs/training/RUN/healer-swarm-a.mmgl.gz
```

Compare candidates against the same frozen presets on both sides; inspect losses
by category before tuning. Repeated games in a mostly deterministic engine are
not independent evidence. Validate changes against saved older clankerbot binaries
with `tools/benchmark.py` and fresh tournament replays too. This roster measures
local robustness, not a tournament win probability.

Checks:

```sh
~/.cargo/bin/cargo test --manifest-path training/Cargo.toml --target-dir target --locked --offline
python3 tools/test_train.py
node tools/analyze-replay.mjs --self-test
```

## Adversarial Team Name stress bots

Fifteen `team-name-*` profiles in category `adversarial` target the vulnerabilities in
[match 794](../logs/scouting/team-name-20260919T124414Z/review.md): an early payload
push can collapse when its healers and reinforcements become separated. These are
hypothetical stronger opponents, not recovered Team Name code.

```sh
python3 tools/train.py run --category adversarial --label adversarial-check
```

Opt-in `coordinated: true` prioritizes critical patients, then patients within the
turnable healing arc, and spreads idle medics among nearby fighters instead of
sending every medic to the foremost fighter. Only enemies within shooting reach
of the patient determine the medic's rear position. `medic_leash` is maximum
walking distance to the nearest healer while an enemy fighter is within range;
zero disables it. A healthy fighter farther away waits near support rather than
feeding forward alone. This does not guarantee continuous healing.
`finish_push: true` releases retreats/leashes and makes all fighters contest once
our mirrored capture reaches +0.8. This deliberately tests finishing a close push
at the cost of protection; it is not universally stronger.

The twelve profiles cover supported advance, mass escort, critical-target focus,
medic hunting, finishing pressure, tight and wide support, heavier healing,
economy buildup, lighter fast escort, base raiding, and splash/strafe pressure.
These remain related implementations. In the initial 24-game check, v12 scored
16 wins and 8 losses with no errors. `team-name-supported` and its finish-push
variant both defeated v12 on both sides. Their identical result ticks in that
run are not four independent pieces of evidence: the finish rule did not produce
a different outcome there. Keep weaker profiles as coverage tests, without
mistaking a larger denominator for a better tournament win-rate estimate.

All original 43 profile objects are unchanged and default the new fields off.
The shared training implementation changed, so use fresh frozen manifests for
future candidate comparisons. The production source/executable remains v12.

Three additional controls isolate the supported profile's changes: `team-name-control`
disables coordination and the medic leash, `team-name-triage-only` enables coordination,
and `team-name-leash-only` enables the leash. V12 won 4/6 control games; the control
and leash-only variants each beat it on side A. Across all fifteen new profiles,
v12 scored **20 wins, 10 losses, no errors**. Coordination alone is not an improvement
in these results; the combined supported configuration is the useful two-sided threat.
The original 43 profiles were rerun: **86/86 wins**, with every outcome, finish tick,
and reason identical to the earlier v12 baseline.

## Independent model opponents

`fable-independent` uses `src/fable.rs`, generated by `claude-fable-5-1` from
[`model-challenge.md`](model-challenge.md), without access to clankerbot's strategy.
Only API compatibility changes were made before evaluation. It anchors the payload,
forms a supporting line, distributes shots, and retreats wounded fighters to medics.
V12 beat its first version on both sides (ticks 3168 and 3073), with no engine errors.
Model diversity adds a different implementation; it does not establish strength.
Independent profiles ignore the shared tactical knobs in their roster entries.

`grok-independent` uses `src/grok.rs`, requested as `cursor-grok-4.6-high` through
the personal Cursor Pro account. It received the same brief with exact Rust API
clarifications. Cursor returned success but did not include a resolved-model ID
in its response metadata. Its two guards, healer protection and endgame miner
contesting are independent tactics. Only `Class`/`BotClass`, a vector dot-product
argument, removal of an unreachable enum branch, and formatting were adapted.
V12 beat this first version on both sides (ticks 2922 and 3008), without errors.

The [initial validation report](../logs/scouting/adversarial-roster-validation.md) links
the frozen runs. Before the full-source expansion, across 60 opponents, v12 had **110 wins and 10 losses**
in the assembled both-side baseline, with no errors; repeated integration checks
are excluded. Treat the new losses as targets for tuning, not a live ranking estimate.

```sh
python3 tools/train.py run --category independent --label independent-check
```

## Counters designed with full source

Fable and Grok each received the complete v12 strategy and tests, public API,
supported-push source and measured failures. They each generated six distinct
patch sets against v12, so these reuse its engine integration and combat helpers.
The exact original prompt, model responses, reviewed patches and provenance are
under `.mm/experiments/whitebox-counters/`. Fable's first response had a truncated
first definition: five complete bots were recovered exactly and a sixth requested
separately. No tactical/API repairs were needed for these twelve implementations.

`fable_counters.rs`, `grok_counters.rs`, and `local_counters.rs` contain the frozen
controllers. They preserve v12's per-match observation memory and ignore the shared
roster knobs. The local group adds eight isolated counter hypotheses. Six additional
`pressure-*` presets exercise the existing supported-push controller's guard count,
spacing, leash, engagement range and economy. `pressure-five-guards` beats v12 on
both sides at ticks 4021/4019; several other variants are weaker and remain coverage
tests rather than evidence of strength.

```sh
python3 tools/train.py run --category whitebox --label full-source-counters
python3 tools/train.py run --profile pressure-five-guards --profile v12-mirror --label pressure-control
```

The mirror control establishes a side effect: identical v12 bots win from A and
lose from B at tick 4132. One-sided wins that follow this pattern do not prove a
counter is stronger. Do not interpret a larger roster as tournament confidence.
The [full-source validation report](../logs/scouting/whitebox-counter-validation.md)
records outcomes, both-side replays, rejected candidate fixes and source hashes.


## IAMABOT v13 retreating miners

Three `approx-iamabot-*` profiles add an opt-in `evade_miners` behavior. Public
matches 982 and 907 show a three-miner/four-healer opening, expansion toward six
miners/eight healers, and miners withdrawing from approaching fighters while
combat units hold the center. The profiles reuse existing combat and vary escort,
medic hunting, and mass capture; the escape vector and switch timing are modeling
choices, not recovered competitor code. Old profiles keep their original behavior.

V12 wins five of their six games; the escort approximation wins from a different
side than the real IAMABOT loss. They expose a stress case but do not reproduce
that live matchup. Use the saved replay-decision regression alongside them:
`.mm/experiments/iamabot/replay-regression.rs` and its four recorded state fixtures.
The ordinary Rust regression also checks blocked-miner versus combat-target
flanking. See the current hourly report for candidate acceptance or rejection.


## Potatoes v26 heavy economy

`approx-potatoes26-hold` and `approx-potatoes26-focus` model eight miners, a
seven-fighter/two-healer opening, support up to seven medics, and a delayed advance.
They reuse the existing safe miner retirement and combat behavior. Fixed departure
at 1100/1600 and retirement at 5000 are stress assumptions: public matches 1074,
1046, 961 and 978 show variable timing and partial miner reductions. These are
composition approximations, not Potatoes' private code. V13 wins all four games,
so those local wins alone do not reproduce or solve the live B-side loss.

Experiments and recorded-state checks: `.mm/experiments/potatoes26/`.

## IAMABOT v15 combat controls

The 94-opponent configurable roster misses the A-side failure against IAMABOT v15.
Two additional saved approximations use our own v14 combat code with its observed
seven-miner/nine-medic composition and alternate targeting assumptions. They are
not competitor code. Source and binaries are frozen under `.mm/opponents/iamabot15-approx-v14-*`.
Run both sides against the frozen v14 baseline and the early-contest release:

```sh
python3 .mm/experiments/iamabot15-counter/spar.py baseline release
python3 .mm/experiments/iamabot15-counter/spar.py --opponent approx-v14-finish baseline release
```

The first model splits against v14 (our A loss/B win). Use these controls in addition
to the main roster and historical saved-version controls. The helper freezes each
binary/hash and enables engine time limits and error checks. New candidate binaries
can be placed under the experiment directory and passed by their filename.

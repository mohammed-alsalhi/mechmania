#!/usr/bin/env python3
"""Build configurable sparring bots and play a candidate on both sides. Stdlib only."""
import argparse
from collections import Counter, defaultdict
from datetime import datetime, timezone
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
CARGO = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
ENGINE = ROOT / ".mm/bin/mm-engine"
BINARY = ROOT / "target/release/training-bot"


def write_json(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


def digest(path):
    checksum = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            checksum.update(chunk)
    return checksum.hexdigest()


def link_profiles(directory, profiles):
    directory.mkdir(parents=True, exist_ok=True)
    shutil.copy2(BINARY, directory / "training-bot")
    for profile in profiles:
        link = directory / profile["name"]
        if link.is_symlink():
            link.unlink()
        link.symlink_to("training-bot")


def build():
    subprocess.run([CARGO, "build", "--manifest-path", "training/Cargo.toml", "--release",
                    "--target-dir", "target", "--locked", "--offline"], cwd=ROOT, check=True)
    # Read the roster back from the executable, preventing config/binary drift.
    profiles = json.loads(subprocess.check_output([str(BINARY), "--list"], text=True))
    link_profiles(ROOT / ".mm/training/bots", profiles)
    return profiles


def read_result(path, side):
    result, last_tick, classes, samples = None, 0, {}, {}
    opponent = "fleet_b" if side == "a" else "fleet_a"
    peak = Counter()
    with path.open() as stream:
        for line in stream:
            if line.startswith("# result: "):
                if result is not None:
                    raise ValueError("duplicate replay result")
                result = json.loads(line[10:])
            elif line.startswith("# time enforcement: disabled"):
                raise ValueError("time enforcement was disabled")
            elif line.strip() and not line.startswith("#"):
                frame = json.loads(line)
                if "tick" not in frame:
                    continue  # Game configuration.
                if frame["tick"] != last_tick + 1:
                    raise ValueError("missing or unordered replay frame")
                last_tick = frame["tick"]
                fleet = frame.get(opponent, {})
                if isinstance(fleet, list):
                    classes = {str(bot["id"]): next(iter(bot["special"])) for bot in fleet}
                else:
                    for bot_id in fleet.get("removed", []):
                        classes.pop(str(bot_id), None)
                    for bot_id, bot in fleet.get("added", {}).items():
                        classes[bot_id] = next(iter(bot["special"]))
                    for bot_id, bot in fleet.get("changed", {}).items():
                        if "special" in bot:
                            classes[bot_id] = next(iter(bot["special"]))
                counts = Counter(classes.values())
                peak |= counts
                if last_tick in (20, 500, 1000, 2000):
                    samples[str(last_tick)] = dict(counts)
    if not result or result.get("winner") not in ("A", "B", None) or result.get("tick") != last_tick or not last_tick:
        raise ValueError("missing or inconsistent completed-match result")
    winner = result["winner"]
    return dict(**result, outcome="draw" if winner is None else "win" if winner.lower() == side else "loss",
                opponent_peak=dict(peak), opponent_samples=samples)


def play(directory, profile, side, timeout):
    name = f"{profile['name']}-{side}"
    replay = directory / f"{name}.mmgl"
    errors = [directory / f"{name}-{team}-stderr.txt" for team in ("a", "b")]
    bots = [directory / "bin/candidate", directory / "bin" / profile["name"]]
    if side == "b":
        bots.reverse()
    command = [str(ENGINE), *map(str, bots), "-o", f"g:{replay}",
               "-o", f"ae:{errors[0]}", "-o", f"be:{errors[1]}"]
    started = time.monotonic()
    row = dict(profile=profile["name"], category=profile["category"], side=side, outcome="error")
    with (directory / f"{name}-engine.txt").open("w") as output:
        process = subprocess.Popen(command, stdout=output, stderr=output, start_new_session=True)
        try:
            process.wait(timeout=timeout)
            if process.returncode:
                raise ValueError(f"engine exit {process.returncode}")
            if any(not path.exists() or path.stat().st_size for path in errors):
                raise ValueError("bot stderr is nonempty or missing; inspect saved stderr files")
            row.update(read_result(replay, side))
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            row["error"] = f"match exceeded {timeout} seconds"
        except (ValueError, OSError, KeyError, TypeError, StopIteration) as error:
            row["error"] = str(error)
        except BaseException:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
            raise
    if replay.exists():
        compressed = replay.with_suffix(".mmgl.gz")
        with replay.open("rb") as source, gzip.open(compressed, "wb", compresslevel=1) as target:
            shutil.copyfileobj(source, target)
        replay.unlink()
        row["replay"] = compressed.name
    row["seconds"] = round(time.monotonic() - started, 2)
    return row


def report(directory, rows, expected):
    totals = Counter(row["outcome"] for row in rows)
    lines = ["# Training roster results", "", f"Completed {len(rows)}/{expected} games: "
             f"{totals['win']} wins, {totals['loss']} losses, {totals['draw']} draws, {totals['error']} errors.", "",
             "Each profile is played on both sides with engine time limits enabled. Errors are not scored as wins or losses.", "",
             "Most presets share a combat implementation; independent profiles use separate controllers. All are frozen for this run. Competitor-inspired profiles approximate observed behavior; "
             "they are not recovered competitor code. Results measure this local roster, not tournament win probability.", "",
             "| Category | Wins | Losses | Draws | Errors |", "|---|---:|---:|---:|---:|"]
    grouped = defaultdict(Counter)
    for row in rows:
        grouped[row["category"]][row["outcome"]] += 1
    for category, counts in sorted(grouped.items()):
        lines.append(f"| {category} | {counts['win']} | {counts['loss']} | {counts['draw']} | {counts['error']} |")
    lines += ["", "| Opponent | Candidate side | Result | Tick | Replay |", "|---|---|---|---:|---|"]
    for row in rows:
        replay = f"[replay]({row['replay']})" if "replay" in row else "—"
        lines.append(f"| {row['profile']} | {row['side'].upper()} | {row['outcome']} | {row.get('tick', '—')} | {replay} |")
    lines += ["", "Binary/source hashes and frozen roster: `manifest.json`, `source/`, and `bin/`. "
              "Fleet samples, per-class peaks, errors, and completion status: `results.json`.", "",
              "Analyze compressed replays with `node tools/analyze-replay.mjs PATH.mmgl.gz --team a` "
              "(use the candidate's recorded side). Decompress a copy for the visualizer with `gzip -dk PATH.mmgl.gz`.", ""]
    (directory / "report.md").write_text("\n".join(lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["list", "build", "run"])
    parser.add_argument("--category", action="append", default=[])
    parser.add_argument("--profile", action="append", default=[])
    parser.add_argument("--candidate", type=Path, default=ROOT / "target/release/bot",
                        help="Existing release binary; this command never rebuilds or submits the candidate")
    parser.add_argument("--label", default="clankerbot")
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.label) or args.timeout <= 0:
        parser.error("label must be letters/numbers/_/- and timeout must be positive")
    profiles = json.loads((ROOT / "training/roster.json").read_text()) if args.command == "list" else build()
    unknown = set(args.profile) - {p["name"] for p in profiles}
    unknown |= set(args.category) - {p["category"] for p in profiles}
    if unknown:
        parser.error(f"unknown profile/category: {', '.join(sorted(unknown))}")
    profiles = [p for p in profiles if (not args.category or p["category"] in args.category)
                and (not args.profile or p["name"] in args.profile)]
    if not profiles:
        parser.error("selection matched no profiles")
    if args.command != "run":
        for p in profiles:
            print(f"{p['category']:10} {p['name']:24} {p['description']}")
        return
    candidate = args.candidate.resolve()
    if not candidate.is_file() or not os.access(candidate, os.X_OK) or not ENGINE.is_file():
        parser.error("build a release candidate and install .mm/bin/mm-engine first")
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    directory = ROOT / "logs/training" / f"{stamp}-{args.label}"
    directory.mkdir(parents=True, exist_ok=False)
    link_profiles(directory / "bin", profiles)
    shutil.copy2(candidate, directory / "bin/candidate")
    sources = ["training/Cargo.toml", "training/Cargo.lock", "training/roster.json",
               "src/core.rs", "tools/train.py"]
    sources += sorted(str(path.relative_to(ROOT)) for path in (ROOT / "training/src").glob("*.rs"))
    hashes = {}
    for source in sources:
        dest = directory / "source" / source
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / source, dest)
        hashes[source] = digest(dest)
    write_json(directory / "manifest.json", dict(created_at=stamp, candidate=str(candidate),
               candidate_sha256=digest(directory / "bin/candidate"), engine=str(ENGINE), engine_sha256=digest(ENGINE),
               training_sha256=digest(directory / "bin/training-bot"), sources=hashes, profiles=profiles, time_limits=True))
    rows = []
    total = len(profiles) * 2
    print(f"Report directory: {directory}", flush=True)
    for profile in profiles:
        for side in ("a", "b"):
            row = play(directory, profile, side, args.timeout)
            rows.append(row)
            write_json(directory / "results.json", rows)
            report(directory, rows, total)
            print(f"{len(rows)}/{total} {profile['name']} side {side}: {row['outcome']} "
                  f"tick {row.get('tick', '?')} ({row['seconds']}s)", flush=True)
    print(f"Report: {directory / 'report.md'}")
    if any(row["outcome"] == "error" for row in rows):
        sys.exit(1)


if __name__ == "__main__":
    main()

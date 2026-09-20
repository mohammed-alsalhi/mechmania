#!/usr/bin/env python3
"""Run the current release bot on both sides against saved local opponents."""
import json
from pathlib import Path
import subprocess
import sys


def main():
    label, *opponents = sys.argv[1:]
    assert label and opponents, "usage: benchmark.py LABEL OPPONENT [OPPONENT ...]"
    results = []
    for opponent in opponents:
        for side in ["a", "b"]:
            name = f"{label}-{opponent}-{side}"
            replay = Path(f"logs/{name}.mmgl")
            errors = Path(f"logs/{name}-errors.txt")
            assert not replay.exists(), f"Refusing to overwrite {replay}"
            bots = ["target/release/bot", f".mm/opponents/{opponent}"]
            if side == "b":
                bots.reverse()
            subprocess.run(
                [".mm/bin/mm-engine", *bots, "-o", f"g:{replay}", "-o", f"ae,be:{errors}"],
                capture_output=True, text=True, check=True,
            )
            result = json.loads(next(
                line.removeprefix("# result: ")
                for line in replay.read_text().splitlines()
                if line.startswith("# result: ")
            ))
            assert not errors.read_text().strip(), f"Bot errors in {errors}"
            results.append(dict(name=name, side=side, **result))
            Path(f"logs/{label}-benchmark.json").write_text(json.dumps(results, indent=2) + "\n")
            print(results[-1], flush=True)


if __name__ == "__main__":
    main()

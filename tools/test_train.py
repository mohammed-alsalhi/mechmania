"""Small replay/parser regression check: python3 tools/test_train.py."""
import gzip
import json
from pathlib import Path
import subprocess
import tempfile

from train import ROOT, digest, read_result, report


with tempfile.TemporaryDirectory() as temporary:
    directory = Path(temporary)
    replay = directory / "example.mmgl"
    frames = [
        {"max_ticks": 9000},
        {"tick": 1, "fleet_a": [], "fleet_b": [{"id": 0, "special": {"Healer": {}}}]},
        {"tick": 2, "fleet_b": {"added": {"1": {"special": {"Battle": {}}}}}},
        {"tick": 3, "fleet_b": {"removed": [0], "changed": {"1": {"health": 7}}}},
    ]
    text = "\n".join(map(json.dumps, frames)) + '\n# result: {"winner":"B","tick":3,"reason":"payload"}\n'
    replay.write_text(text)
    result = read_result(replay, "a")
    assert result["outcome"] == "loss"
    assert result["opponent_peak"] == {"Healer": 1, "Battle": 1}
    assert read_result(replay, "b")["outcome"] == "win"
    replay.write_text(text.replace('"winner":"B"', '"winner":null'))
    assert read_result(replay, "a")["outcome"] == "draw"
    for malformed in (text.replace('"tick":3,"reason"', '"tick":4,"reason"'),
                      text.split("# result:")[0], "# time enforcement: disabled\n" + text,
                      text + '# result: {"winner":"A","tick":3}\n'):
        replay.write_text(malformed)
        try:
            read_result(replay, "a")
        except ValueError:
            pass
        else:
            raise AssertionError("incomplete/invalid replay accepted")
    report(directory, [dict(profile="test", category="healing", side="a", outcome="error")], 2)
    assert "0 wins, 0 losses, 0 draws, 1 errors" in (directory / "report.md").read_text()
    replay.write_bytes(b"abc")
    assert digest(replay) == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    compressed = directory / "example.mmgl.gz"
    with gzip.open(compressed, "wb") as target:
        target.write(text.encode())
    assert gzip.decompress(compressed.read_bytes()).decode() == text

unknown = subprocess.run(["python3", str(ROOT / "tools/train.py"), "list", "--profile", "not-a-profile"], capture_output=True)
assert unknown.returncode != 0 and b"unknown profile" in unknown.stderr
print("Training checks passed: sides, draws, sparse fleet counts, corrupt logs, errors, hashes, compression, invalid selections.")

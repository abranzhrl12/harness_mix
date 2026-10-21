"""Small fixture model for invalidation scope; not a benchmark of the Rust engine."""
import json
from pathlib import Path


ROOT = Path(__file__).parent
data = json.loads((ROOT / "fixtures.json").read_text(encoding="utf-8"))
imports = data["importers"]
all_importers = {item["file"] for item in imports}


def select(event):
    """Illustrate a safe dependency-index policy for the listed event classes."""
    changed = event["changed"]
    kind = event["kind"]
    if kind == "config":
        # A config snapshot should record which importers consumed it.
        return {item["file"] for item in imports if item["module"].startswith("@")}

    # Existing incoming edges cover deletion and export changes; this fixture's
    # target field represents the indexed resolution edge.
    selected = {item["file"] for item in imports if item["target"] == changed}

    # A new path can satisfy a prior negative probe, or alter the candidate set
    # for a currently successful specifier. These are distinct dependencies.
    if kind == "add":
        if changed == "src/missing.ts":
            selected |= {item["file"] for item in imports if item["unresolved"] and item["module"] == "./missing"}
        if changed == "src/lib.js":
            selected |= {item["file"] for item in imports if item["module"] == "./lib"}

    # Recheck reverse importers through barrel modules, to a fixed point.
    targets = {changed}
    while True:
        parents = {item["file"] for item in imports if item["target"] in targets}
        delta = parents - targets
        if not delta:
            break
        selected |= delta
        targets |= delta
    return selected


def main():
    print("event | global candidates | selective candidates | expected included")
    failures = 0
    for event in data["events"]:
        selected = select(event)
        expected = set(event["expected"])
        covered = expected <= selected
        failures += not covered
        print(f"{event['name']} | {len(all_importers)} | {len(selected)} | {covered}")
        assert covered, f"missed {sorted(expected - selected)} for {event['name']}"
    print(f"TOTAL | {len(data['events']) * len(all_importers)} | "
          f"{sum(len(select(e)) for e in data['events'])} | "
          f"{len(data['events'])} scenarios covered, {failures} failures")
    print("Note: fixture counts are not runtime or database benchmark measurements.")


if __name__ == "__main__":
    main()

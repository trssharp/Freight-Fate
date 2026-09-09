"""Select dev for a scheduled legacy build, if this repository carries it."""

import os
import subprocess
from pathlib import Path


def select_branch(event: str) -> bool:
    """Leave tags/manual refs intact; missing dev is only a scheduled no-op.

    Checkout must have fetched all remote branches first. Repository and
    checkout errors still fail the build instead of being mistaken for an
    absent release line.
    """
    if event != "schedule":
        return True
    ref = "refs/remotes/origin/dev"
    result = subprocess.run(["git", "show-ref", "--verify", "--quiet", ref], check=False)
    if result.returncode == 1:
        print("No dev release branch in this repository; no legacy nightly to build.")
        return False
    result.check_returncode()
    subprocess.run(["git", "checkout", "--detach", ref], check=True)
    return True


if __name__ == "__main__":
    available = select_branch(os.environ["GITHUB_EVENT_NAME"])
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as output:
        output.write(f"available={str(available).lower()}\n")

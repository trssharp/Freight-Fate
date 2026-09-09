import subprocess

import pytest
from select_nightly_branch import select_branch


@pytest.fixture
def repository(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    subprocess.run(["git", "init", "-q"], check=True)
    subprocess.run(
        [
            "git",
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "Career baseline",
        ],
        check=True,
    )
    return subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()


def test_career_only_fork_has_no_legacy_nightly(repository):
    assert select_branch("schedule") is False
    assert subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip() == repository


def test_scheduled_build_checks_out_dev_when_present(repository):
    subprocess.run(["git", "update-ref", "refs/remotes/origin/dev", repository], check=True)
    subprocess.run(
        [
            "git",
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-qm",
            "Newer career work",
        ],
        check=True,
    )
    assert select_branch("schedule") is True
    assert subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip() == repository


@pytest.mark.parametrize("event", ["push", "workflow_dispatch"])
def test_explicit_build_keeps_its_ref_without_dev(repository, event):
    assert select_branch(event) is True
    assert subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip() == repository


def test_broken_checkout_is_not_treated_as_a_missing_branch(tmp_path, monkeypatch):
    monkeypatch.chdir(tmp_path)
    with pytest.raises(subprocess.CalledProcessError):
        select_branch("schedule")

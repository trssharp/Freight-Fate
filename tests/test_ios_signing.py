"""tools/ios_signing.py: JWT shape, stored-certificate lookup, profile reuse, cleanup."""

from __future__ import annotations

import base64
import importlib.util
import json
import shutil
import subprocess
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("ios_signing", ROOT / "tools" / "ios_signing.py")
ios_signing = importlib.util.module_from_spec(spec)
sys.modules["ios_signing"] = ios_signing
spec.loader.exec_module(ios_signing)

needs_openssl = pytest.mark.skipif(shutil.which("openssl") is None, reason="no openssl")


def _der(r: int, s: int) -> bytes:
    def enc(n: int) -> bytes:
        raw = n.to_bytes((n.bit_length() + 7) // 8 or 1, "big")
        if raw[0] & 0x80:
            raw = b"\x00" + raw
        return b"\x02" + bytes([len(raw)]) + raw

    body = enc(r) + enc(s)
    return b"\x30" + bytes([len(body)]) + body


def test_der_to_raw_pads_and_strips() -> None:
    r, s = 0x80 << 248, 5  # r needs a leading zero in DER, s is short
    raw = ios_signing.der_to_raw_signature(_der(r, s))
    assert len(raw) == 64
    assert int.from_bytes(raw[:32], "big") == r
    assert int.from_bytes(raw[32:], "big") == s


def test_der_to_raw_rejects_junk() -> None:
    with pytest.raises(ios_signing.SigningError):
        ios_signing.der_to_raw_signature(b"nope")


def test_token_claims(monkeypatch, tmp_path) -> None:
    monkeypatch.setattr(ios_signing, "sign_es256", lambda data, path: b"\x01" * 64)
    token = ios_signing.make_token("KEY123", "issuer-1", tmp_path / "k.p8", now=1000)
    head, payload, sig = token.split(".")

    def dec(part: str) -> dict:
        return json.loads(base64.urlsafe_b64decode(part + "=" * (-len(part) % 4)))

    assert dec(head) == {"alg": "ES256", "kid": "KEY123", "typ": "JWT"}
    claims = dec(payload)
    assert claims["iss"] == "issuer-1" and claims["aud"] == "appstoreconnect-v1"
    assert claims["exp"] - claims["iat"] <= 20 * 60
    assert len(base64.urlsafe_b64decode(sig + "==")) == 64


@needs_openssl
def test_real_openssl_signature_is_64_bytes(tmp_path) -> None:
    key = tmp_path / "k.p8"
    subprocess.run(
        ["openssl", "genpkey", "-algorithm", "EC", "-pkeyopt", "ec_paramgen_curve:P-256",
         "-out", str(key)],
        check=True, capture_output=True,
    )  # fmt: skip
    assert len(ios_signing.sign_es256(b"abc", key)) == 64


@needs_openssl
def test_certificate_serial_reads_pem(tmp_path) -> None:
    key, cert = tmp_path / "k.pem", tmp_path / "c.pem"
    subprocess.run(
        ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", str(key),
         "-out", str(cert), "-subj", "/CN=t", "-days", "1", "-set_serial", "0x00AB12"],
        check=True, capture_output=True,
    )  # fmt: skip
    assert ios_signing.certificate_serial(cert) == "AB12"


SERIAL = "6FAEDF0892775C53B5B9AE78C662511B"
BUNDLE_ID_RESOURCE = "B1"


def _cert(serial=SERIAL, kind="DISTRIBUTION", expires="2099-01-01T00:00:00.000+00:00"):
    return {
        "id": "C1",
        "attributes": {
            "serialNumber": serial,
            "certificateType": kind,
            "expirationDate": expires,
        },
    }


def _profile(pid, bundle="B1", cert="C1", expires="2099-01-01T00:00:00.000+00:00"):
    return {
        "id": pid,
        "attributes": {
            "name": f"profile {pid}",
            "expirationDate": expires,
            "profileContent": base64.b64encode(pid.encode()).decode(),
        },
        "relationships": {
            "bundleId": {"data": {"type": "bundleIds", "id": bundle}},
            "certificates": {"data": [{"type": "certificates", "id": cert}]},
        },
    }


class FakeApi:
    def __init__(self, bundles=None, fail=None):
        self.calls: list[tuple[str, str, dict | None]] = []
        self.bundles = bundles if bundles is not None else []
        self.certs = [_cert()]
        self.profiles: list[dict] = []
        self.fail = fail or {}

    def __call__(self, token, method, path, body=None):
        self.calls.append((method, path.split("?")[0], body))
        key = (method, path.split("?")[0])
        if key in self.fail:
            raise self.fail[key]
        if key == ("GET", "/v1/bundleIds"):
            return {"data": self.bundles}
        if key == ("GET", "/v1/certificates"):
            return {"data": self.certs}
        if key == ("GET", "/v1/profiles"):
            return {"data": self.profiles}
        if key == ("POST", "/v1/bundleIds"):
            return {"data": {"id": BUNDLE_ID_RESOURCE}}
        if key == ("POST", "/v1/profiles"):
            prof = base64.b64encode(b"profilebytes").decode()
            return {"data": {"id": "P1", "attributes": {"profileContent": prof}}}
        if key == ("GET", "/v1/apps"):
            return {"data": self.bundles}
        return {}


@pytest.fixture
def api(monkeypatch, tmp_path):
    fake = FakeApi()
    monkeypatch.setattr(ios_signing, "api_request", fake)
    monkeypatch.setattr(ios_signing, "_token", lambda: "tok")
    monkeypatch.setattr(ios_signing, "certificate_serial", lambda path: SERIAL)
    (tmp_path / ios_signing.CERT_FILE).write_text("cert")
    return fake


def _methods(api) -> set[str]:
    return {c[0] for c in api.calls}


def test_prepare_never_creates_or_revokes_a_certificate(api, tmp_path) -> None:
    assert ios_signing.main(["prepare", "--out", str(tmp_path)]) == 0
    touched = [(c[0], c[1]) for c in api.calls]
    assert ("POST", "/v1/certificates") not in touched
    assert not any(m == "DELETE" for m, _ in touched)


def test_prepare_makes_one_profile_when_none_is_active(api, tmp_path) -> None:
    assert ios_signing.main(["prepare", "--out", str(tmp_path)]) == 0
    posts = [(c[1], c[2]) for c in api.calls if c[0] == "POST"]
    assert [p[0] for p in posts] == ["/v1/bundleIds", "/v1/profiles"]
    rel = posts[1][1]["data"]["relationships"]
    assert rel["bundleId"]["data"]["id"] == BUNDLE_ID_RESOURCE
    assert rel["certificates"]["data"][0]["id"] == "C1"
    assert posts[1][1]["data"]["attributes"]["profileType"] == "IOS_APP_STORE"
    assert (tmp_path / ios_signing.PROFILE_FILE).read_bytes() == b"profilebytes"


def test_prepare_reuses_the_profile_for_the_stored_cert(api, tmp_path) -> None:
    api.bundles = [{"id": "B1", "attributes": {"identifier": ios_signing.BUNDLE_ID}}]
    api.profiles = [
        _profile("OTHER_CERT", cert="C9"),
        _profile("OTHER_APP", bundle="B9"),
        _profile("OLD", expires="2027-01-01T00:00:00.000+00:00"),
        _profile("NEW", expires="2028-01-01T00:00:00.000+00:00"),
    ]
    assert ios_signing.main(["prepare", "--out", str(tmp_path)]) == 0
    assert _methods(api) == {"GET"}
    assert (tmp_path / ios_signing.PROFILE_FILE).read_bytes() == b"NEW"


def test_prepare_ignores_prefix_bundle_matches(api, tmp_path) -> None:
    api.bundles = [{"id": "X", "attributes": {"identifier": ios_signing.BUNDLE_ID + ".extra"}}]
    ios_signing.main(["prepare", "--out", str(tmp_path)])
    assert ("POST", "/v1/bundleIds") in [(c[0], c[1]) for c in api.calls]


def test_prepare_stops_when_the_stored_cert_is_gone(api, tmp_path, capsys) -> None:
    api.certs = [_cert(serial="1234")]
    assert ios_signing.main(["prepare", "--out", str(tmp_path)]) == 1
    out = capsys.readouterr().out
    assert "not on the team" in out and "IOS_DISTRIBUTION_P12_BASE64" in out
    assert _methods(api) == {"GET"}


def test_prepare_stops_when_the_stored_cert_expired(api, tmp_path, capsys) -> None:
    api.certs = [_cert(expires="2020-01-01T00:00:00.000+00:00")]
    assert ios_signing.main(["prepare", "--out", str(tmp_path)]) == 1
    assert "expired on 2020-01-01" in capsys.readouterr().out


def test_prepare_refuses_a_development_cert(api, tmp_path, capsys) -> None:
    api.certs = [_cert(kind="DEVELOPMENT")]
    assert ios_signing.main(["prepare", "--out", str(tmp_path)]) == 1
    assert "not a distribution one" in capsys.readouterr().out


def test_prepare_without_the_exported_cert(api, tmp_path, capsys) -> None:
    (tmp_path / ios_signing.CERT_FILE).unlink()
    assert ios_signing.main(["prepare", "--out", str(tmp_path)]) == 1
    assert "IOS_DISTRIBUTION_P12_PASSWORD" in capsys.readouterr().out
    assert api.calls == []


def test_forbidden_key_gets_plain_message(api, tmp_path, capsys) -> None:
    err = ios_signing.ApiError("GET", "/v1/bundleIds", 403, "no")
    api.fail[("GET", "/v1/bundleIds")] = err
    assert ios_signing.main(["prepare", "--out", str(tmp_path)]) == 1
    out = capsys.readouterr().out
    assert "App Manager or Admin" in out


def test_unauthorized_key_message(api, tmp_path, capsys) -> None:
    api.fail[("GET", "/v1/certificates")] = ios_signing.ApiError("GET", "/v1/certificates", 401, "")
    assert ios_signing.main(["prepare", "--out", str(tmp_path)]) == 1
    assert "ASC_KEY_P8_BASE64" in capsys.readouterr().out


def test_missing_app_record_says_what_to_do(api, capsys) -> None:
    assert ios_signing.main(["check-app"]) == 1
    out = capsys.readouterr().out
    assert "net.orinks.freightfate" in out and "must match" in out


def test_app_record_present(api, capsys) -> None:
    api.bundles = [{"id": "A"}]
    assert ios_signing.main(["check-app"]) == 0


def test_cleanup_only_removes_local_files(api, tmp_path) -> None:
    for name in (ios_signing.CERT_FILE, ios_signing.PROFILE_FILE, ios_signing.P12_FILE):
        (tmp_path / name).write_text("x")
    assert ios_signing.main(["cleanup", "--out", str(tmp_path)]) == 0
    assert api.calls == []
    assert list(tmp_path.iterdir()) == []


def test_cleanup_with_nothing_there_is_quiet(tmp_path) -> None:
    assert ios_signing.main(["cleanup", "--out", str(tmp_path)]) == 0


def test_workflow_signs_with_the_stored_certificate() -> None:
    workflow = (ROOT / ".github" / "workflows" / "ios-testflight.yml").read_text(encoding="utf-8")
    assert "secrets.IOS_DISTRIBUTION_P12_BASE64" in workflow
    assert "secrets.IOS_DISTRIBUTION_P12_PASSWORD" in workflow
    assert "--label" not in workflow


def test_bundle_id_comes_from_build_ios() -> None:
    assert ios_signing.BUNDLE_ID == ios_signing._BUILD_IOS.BUNDLE_ID


def test_next_build_continues_after_highest() -> None:
    builds = [("1.9.0", "7"), ("1.9.0", "12"), ("1.8.0", "1.9.0.dev0")]
    assert ios_signing.next_build_number(builds, "1.9.0", 3) == (13, 12)


def test_next_build_never_below_run_number() -> None:
    assert ios_signing.next_build_number([("1.9.0", "4")], "1.9.0", 30) == (31, 4)
    assert ios_signing.next_build_number([], "1.9.0", 0) == (1, 0)


def test_next_build_refuses_lower_version() -> None:
    with pytest.raises(ios_signing.SigningError, match="higher than this build"):
        ios_signing.next_build_number([("1.9.1", "2")], "1.9.0", 1)


def test_workflow_leaves_export_compliance_to_info_plist() -> None:
    """Setting it again after upload fails with HTTP 409 (value already set)."""
    workflow = (ROOT / ".github" / "workflows" / "ios-testflight.yml").read_text(encoding="utf-8")
    active = [ln for ln in workflow.splitlines() if not ln.lstrip().startswith("#")]
    assert not any("uses-non-exempt-encryption" in ln for ln in active)
    build_ios = (ROOT / "tools" / "build_ios.py").read_text(encoding="utf-8")
    assert '"ITSAppUsesNonExemptEncryption": False' in build_ios


class FakeTestFlight:
    """App Store Connect as seen by distribute: one app, its groups and builds."""

    def __init__(self, states=("VALID",), submitted=False):
        self.calls: list[tuple[str, str, dict | None]] = []
        self.states = list(states)
        self.submitted = submitted
        self.groups = [
            {"id": "G-int", "attributes": {"name": "Friends", "isInternalGroup": True}},
            {"id": "G-ext", "attributes": {"name": "Friends", "isInternalGroup": False}},
        ]

    def __call__(self, token, method, path, body=None):
        bare = path.split("?")[0]
        self.calls.append((method, bare, body))
        if bare == "/v1/apps":
            return {"data": [{"id": "APP"}]}
        if bare == "/v1/apps/APP/betaGroups":
            return {"data": self.groups}
        if bare == "/v1/builds":
            assert "filter%5Bversion%5D=10" in path
            state = self.states.pop(0) if len(self.states) > 1 else self.states[0]
            return {"data": [{"id": "BUILD", "attributes": {"processingState": state}}]}
        if bare == "/v1/builds/BUILD/betaAppReviewSubmission":
            return {"data": {"id": "S"} if self.submitted else None}
        return {}


@pytest.fixture
def testflight(monkeypatch):
    fake = FakeTestFlight()
    monkeypatch.setattr(ios_signing, "api_request", fake)
    monkeypatch.setattr(ios_signing, "_token", lambda: "tok")
    monkeypatch.setattr(ios_signing.time, "sleep", lambda seconds: None)
    return fake


def _writes(fake) -> list[tuple[str, str, dict | None]]:
    return [c for c in fake.calls if c[0] != "GET"]


def test_distribute_adds_to_the_external_group_and_submits(testflight, capsys) -> None:
    assert ios_signing.main(["distribute", "--build-number", "10"]) == 0
    added, submitted = _writes(testflight)
    assert added[1] == "/v1/betaGroups/G-ext/relationships/builds"
    assert added[2] == {"data": [{"type": "builds", "id": "BUILD"}]}
    assert submitted[1] == "/v1/betaAppReviewSubmissions"
    assert submitted[2]["data"]["relationships"]["build"]["data"]["id"] == "BUILD"
    assert "submitted for beta review" in capsys.readouterr().out


def test_distribute_waits_for_processing(testflight) -> None:
    testflight.states = ["PROCESSING", "PROCESSING", "VALID"]
    assert ios_signing.main(["distribute", "--build-number", "10"]) == 0
    assert sum(1 for c in testflight.calls if c[1] == "/v1/builds") == 3


def test_distribute_does_not_resubmit(testflight, capsys) -> None:
    testflight.submitted = True
    assert ios_signing.main(["distribute", "--build-number", "10"]) == 0
    assert [c[1] for c in _writes(testflight)] == ["/v1/betaGroups/G-ext/relationships/builds"]
    assert "already submitted" in capsys.readouterr().out


def test_distribute_stops_on_a_failed_build(testflight, capsys) -> None:
    testflight.states = ["INVALID"]
    assert ios_signing.main(["distribute", "--build-number", "10"]) == 1
    assert _writes(testflight) == []
    assert "INVALID" in capsys.readouterr().out


def test_distribute_needs_an_external_group(testflight, capsys) -> None:
    testflight.groups = testflight.groups[:1]
    assert ios_signing.main(["distribute", "--build-number", "10"]) == 1
    assert _writes(testflight) == []
    assert "no external TestFlight group named 'Friends'" in capsys.readouterr().out


def test_workflow_sends_real_uploads_to_friends_by_default() -> None:
    workflow = (ROOT / ".github" / "workflows" / "ios-testflight.yml").read_text(encoding="utf-8")
    assert "inputs.upload == true && inputs.testers == 'friends'" in workflow
    assert "tools/ios_signing.py distribute" in workflow
    assert "default: friends" in workflow
